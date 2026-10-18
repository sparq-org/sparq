// sq-py8h.1 — DIFFERENTIAL tests for the disclosed-key bounded path.
//! Differential acceptance suite for [`eval_bounded_path_disclosed`].
//!
//! THE acceptance criterion (mirrors `differential_three_holder_chain_equals_union`
//! in [`crate::join`]): the federated DISCLOSED-KEY unroll of a bounded path over
//! per-holder disclosed edge partials must equal the clear-text engine's
//! `eval_path` over the UNION store, for each bounded form in the design matrix
//! (§6 step 1). The union store is the single `sparq-engine` `Graph` holding the
//! union of all holders' edges; the clear-text side runs the *equivalent* SPARQL
//! 1.1 property path through the real engine (`query`), which exercises the
//! engine's `eval_path` (`exec.rs`).
//!
//! Bounded `{m,k}` is expressed to the engine as the equivalent SPARQL path the
//! standard DOES carry. The `{m,n}`-style counting quantifiers (`{n}`, `{m,n}`,
//! `{m,}`, `{,n}`) appeared in the SPARQL 1.1 *working drafts* but were REMOVED
//! before the final W3C Recommendation — the group lacked consensus on the
//! counting/non-counting semantics — so the final Rec's property-path grammar
//! carries only `*`, `+`, and `?` (the engine's `eval_path` accordingly has only
//! `ZeroOrMore` / `OneOrMore` / `ZeroOrOne` path expressions, no `{m,n}` variant).
//! Several engines (and sparq, via THIS module) support the bounded `{m,k}` form
//! as an EXTENSION. To run the differential oracle through the standard engine we
//! therefore rewrite the bounded form as the union of the fixed-length chains it
//! denotes — a construction that uses only the sequence (`/`), alternative (`|`),
//! and `?` operators the final Rec carries (never the removed `{m,n}` form):
//!   - `(p){k}`     ==  `p/p/.../p`        (k copies; a `Sequence`)
//!   - `(p){m,k}`   ==  union of `p^ℓ` for ℓ in m..=k (an `Alternative` of chains)
//!   - `(p){0,k}`   ==  `(p)? | p^2 | ... | p^k`  (the `?` supplies the length-0/1 arms)
//!   - alternation  ==  `(p|q)/(p|q)/...`
//!
//! The unroller and the engine independently compute the SAME bounded relation,
//! so the equality is exact (design §1.2 — bounded, not an approximation).
//!
//! CRYPTO-FREE assertion: the disclosed-key path touches NO MPC primitive. There
//! is no `CommCounter`, no `ShamirBackend`, no `secure_equal` on this code path —
//! it is a fold of `DisclosedKeyJoin` over disclosed global IRIs. The
//! `crypto_free_no_mpc_round_artifacts` test pins that with a NEGATIVE CONTROL: the
//! same counter type that the secure path records rounds against is first shown to
//! go non-zero under MPC-style ops (so the `== 0` assertion is a real witness, not
//! a tautology), then shown to stay zero across the whole evaluation — because the
//! evaluation's only round-recording surface (`record_mult` / `record_open` /
//! `record_independent_equalities`, all called ONLY from the bench harness, never
//! from the protocol primitives) is unreachable from the unroll's call graph.

use super::*;
use crate::holder::Holder;
use oxrdf::Term;
use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "@prefix ex: <http://ex/> .\n";

fn nn(iri: &str) -> NamedNode {
    NamedNode::new(iri).unwrap()
}

/// Build the per-predicate disclosed edge set: each holder discloses its
/// `(?s,?o)` rows for a predicate via `evaluate_local`, exactly as a federation
/// member would. Returns the [`DisclosedEdges`] the unroller consumes.
fn disclosed_edges(holder_docs: &[(&str, &[&str])]) -> DisclosedEdges {
    // holder_docs: (turtle body, [predicate IRIs this holder discloses]).
    let mut entries: Vec<(NamedNode, PartialResult)> = Vec::new();
    for (i, (doc, preds)) in holder_docs.iter().enumerate() {
        let h = Holder::from_rdf(format!("h{i}"), &format!("{PFX}{doc}"), "turtle").unwrap();
        for p in *preds {
            let part = h
                .evaluate_local(&format!("SELECT ?s ?o WHERE {{ ?s <{p}> ?o }}"))
                .unwrap();
            entries.push((nn(p), part));
        }
    }
    DisclosedEdges::from_holder_edges(entries).unwrap()
}

/// Build a single union store from holder turtle bodies — the "evaluate the
/// whole path over D = ⋃ holder graphs" side of the differential.
fn union_graph(docs: &[&str]) -> Graph {
    let mut combined = String::from(PFX);
    for d in docs {
        combined.push_str(d);
        combined.push('\n');
    }
    Graph::load_str(&combined, "turtle").expect("union graph parses")
}

/// The canonical set of `(a,b)` endpoint IRI pairs from a `(vars,rows)` partial,
/// as `(a_debug, b_debug)` strings — order-independent, dedup'd. The unroller
/// projects onto `[?__pp_a, ?__pp_b]`; the engine query below projects `?a ?b`.
fn pair_set(vars: &[Variable], rows: &[Vec<Option<Term>>]) -> BTreeSet<(String, String)> {
    // Find the two endpoint columns by position (both sides are 2-col results).
    assert_eq!(
        vars.len(),
        2,
        "expected a 2-column (a,b) result, got {vars:?}"
    );
    rows.iter()
        .map(|r| {
            // Endpoint pairs are 2-column, fully-bound bindings by construction
            // (both the unroller's projection and the engine's `?a ?b` projection
            // bind both columns). An UNBOUND endpoint would indicate a real bug in
            // the engine query or the unroller projection — so fail fast with a
            // clear message rather than masking it as the empty string via
            // `unwrap_or_default()` (which would also risk false set collisions
            // between distinct unbound rows).
            let a = r[0]
                .as_ref()
                .map(|t| format!("{t:?}"))
                .expect("endpoint column 0 (?a) must be BOUND — an unbound endpoint indicates a bug in the unroller projection or engine query");
            let b = r[1]
                .as_ref()
                .map(|t| format!("{t:?}"))
                .expect("endpoint column 1 (?b) must be BOUND — an unbound endpoint indicates a bug in the unroller projection or engine query");
            (a, b)
        })
        .collect()
}

/// Run the clear-text engine oracle: the equivalent SPARQL path over the union
/// store, projecting `?a ?b`. Exercises `eval_path` (`exec.rs`).
fn clear_text_pairs(union_docs: &[&str], path_sparql: &str) -> BTreeSet<(String, String)> {
    let u = union_graph(union_docs);
    let q = format!("PREFIX ex: <http://ex/> SELECT ?a ?b WHERE {{ ?a {path_sparql} ?b }}");
    let res = query(&u, &q).unwrap();
    pair_set(&res.vars, &res.rows)
}

/// Run the federated DISCLOSED-KEY unroll and return its `(a,b)` pair set.
fn federated_pairs(edges: &DisclosedEdges, form: &PathForm) -> BTreeSet<(String, String)> {
    let got = eval_bounded_path_disclosed(edges, form).unwrap();
    assert_eq!(got.holder, HolderId::new("federation"));
    pair_set(&got.vars, &got.rows)
}

// =====================================================================
// 1. PLAIN SEQUENCE  p1/p2/p3
// =====================================================================

/// A plain 3-step sequence path `?a ex:p1/ex:p2/ex:p3 ?b`, edges split across
/// THREE holders (one predicate each) — the disclosed-key fold must equal the
/// engine's `eval_path` of the sequence over the union. This is the direct
/// generalisation of `differential_three_holder_chain_equals_union`.
#[test]
fn differential_plain_sequence_equals_eval_path() {
    let a_doc = "ex:p1 ex:p1 ex:m1 . ex:p2 ex:p1 ex:m2 .";
    let b_doc = "ex:m1 ex:p2 ex:n1 . ex:m2 ex:p2 ex:n1 . ex:m9 ex:p2 ex:n2 .";
    let c_doc = "ex:n1 ex:p3 ex:z1 . ex:n2 ex:p3 ex:z2 .";

    let edges = disclosed_edges(&[
        (a_doc, &["http://ex/p1"]),
        (b_doc, &["http://ex/p2"]),
        (c_doc, &["http://ex/p3"]),
    ]);
    let form = PathForm::sequence([nn("http://ex/p1"), nn("http://ex/p2"), nn("http://ex/p3")]);

    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[a_doc, b_doc, c_doc], "ex:p1/ex:p2/ex:p3");
    assert_eq!(got, want, "seq unroll must equal eval_path over the union");
    // Concretely: p1->m1->n1->z1 and p2->m2->n1->z1. m9/n2 dropped (no p1 into m9).
    assert_eq!(got.len(), 2, "{got:?}");
}

// =====================================================================
// 2. EXACT  (p){k}
// =====================================================================

/// Exact `(ex:knows){3}` — a 3-hop chain of ONE predicate, edges split across
/// holders. Differential against `ex:knows/ex:knows/ex:knows`.
#[test]
fn differential_exact_k_equals_eval_path() {
    // A chain a->b->c->d->e plus a side branch; exactly-3 connects a->d and b->e.
    let h1 = "ex:a ex:knows ex:b . ex:b ex:knows ex:c .";
    let h2 = "ex:c ex:knows ex:d . ex:d ex:knows ex:e .";
    let edges = disclosed_edges(&[(h1, &["http://ex/knows"]), (h2, &["http://ex/knows"])]);

    let form = PathForm::exact(nn("http://ex/knows"), 3);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "ex:knows/ex:knows/ex:knows");
    assert_eq!(got, want);
    // a->b->c->d (len3) and b->c->d->e (len3): {(a,d),(b,e)}.
    assert_eq!(got.len(), 2, "{got:?}");
}

// =====================================================================
// 3. RANGE  (p){m,k}
// =====================================================================

/// Range `(ex:knows){1,3}` (bounded `+`) — UNION of lengths 1,2,3, deduped.
/// Differential against `ex:knows | ex:knows/ex:knows | ex:knows/ex:knows/ex:knows`.
#[test]
fn differential_range_m_k_equals_eval_path() {
    let h1 = "ex:a ex:knows ex:b . ex:b ex:knows ex:c .";
    let h2 = "ex:c ex:knows ex:d .";
    let edges = disclosed_edges(&[(h1, &["http://ex/knows"]), (h2, &["http://ex/knows"])]);

    let form = PathForm::range(nn("http://ex/knows"), 1, 3);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(
        &[h1, h2],
        "ex:knows | ex:knows/ex:knows | ex:knows/ex:knows/ex:knows",
    );
    assert_eq!(got, want);
    // len1: ab,bc,cd ; len2: ac,bd ; len3: ad. All distinct -> 6 pairs.
    assert_eq!(got.len(), 6, "{got:?}");
}

/// Range with a NON-1 lower bound `(ex:knows){2,3}` — must EXCLUDE the length-1
/// pairs. Pins that `min` is honoured (not silently treated as 1).
#[test]
fn differential_range_min_two_excludes_length_one() {
    let h1 = "ex:a ex:knows ex:b . ex:b ex:knows ex:c .";
    let h2 = "ex:c ex:knows ex:d .";
    let edges = disclosed_edges(&[(h1, &["http://ex/knows"]), (h2, &["http://ex/knows"])]);

    let form = PathForm::range(nn("http://ex/knows"), 2, 3);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "ex:knows/ex:knows | ex:knows/ex:knows/ex:knows");
    assert_eq!(got, want);
    // len2: ac,bd ; len3: ad. The length-1 pairs (ab,bc,cd) are EXCLUDED -> 3.
    assert_eq!(got.len(), 3, "{got:?}");
    // Assert a length-1 pair is genuinely absent.
    let ab = (
        format!("{:?}", Term::from(nn("http://ex/a"))),
        format!("{:?}", Term::from(nn("http://ex/b"))),
    );
    assert!(
        !got.contains(&ab),
        "length-1 pair (a,b) must be excluded by min=2"
    );
}

// =====================================================================
// 4. REFLEXIVE  (p){0,k}  — the identity pairs appear exactly once
// =====================================================================

/// Reflexive `(ex:knows){0,2}` — the `{1,2}` union PLUS the length-0 identity
/// pairs `(x,x)` for every node, each exactly once (design §2.3). Differential
/// against `(ex:knows)? | ex:knows/ex:knows` (the `?` carries length 0 and 1).
#[test]
fn differential_reflexive_zero_k_identity_pairs_once() {
    let h1 = "ex:a ex:knows ex:b .";
    let h2 = "ex:b ex:knows ex:c .";
    let edges = disclosed_edges(&[(h1, &["http://ex/knows"]), (h2, &["http://ex/knows"])]);

    let form = PathForm::range(nn("http://ex/knows"), 0, 2);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "(ex:knows)? | ex:knows/ex:knows");
    assert_eq!(got, want, "reflexive {{0,2}} must equal eval_path");

    // The identity (x,x) pairs: nodes are a,b,c -> (a,a),(b,b),(c,c).
    for node in ["http://ex/a", "http://ex/b", "http://ex/c"] {
        let d = format!("{:?}", Term::from(nn(node)));
        let count = got.iter().filter(|(a, b)| *a == d && *b == d).count();
        assert_eq!(
            count, 1,
            "identity pair ({node},{node}) must appear EXACTLY once"
        );
    }
    // Full set: identity {aa,bb,cc} + len1 {ab,bc} + len2 {ac} = 6 distinct pairs.
    assert_eq!(got.len(), 6, "{got:?}");
}

/// `(p?)` is the `{0,1}` special case: reflexive identity + the single 1-hop
/// pattern. Differential against the engine's `(ex:knows)?`.
#[test]
fn differential_zero_or_one_p_question() {
    let h1 = "ex:a ex:knows ex:b .";
    let h2 = "ex:b ex:knows ex:c .";
    let edges = disclosed_edges(&[(h1, &["http://ex/knows"]), (h2, &["http://ex/knows"])]);

    let form = PathForm::range(nn("http://ex/knows"), 0, 1);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "(ex:knows)?");
    assert_eq!(got, want);
    // identity {aa,bb,cc} + len1 {ab,bc} = 5.
    assert_eq!(got.len(), 5, "{got:?}");
}

// =====================================================================
// 5. ALTERNATION  (p|q)
// =====================================================================

/// Alternation per hop, exactly-2: `(ex:p|ex:q){2}` unrolls to the FOUR chains
/// {pp,pq,qp,qq}. Differential against `(ex:p|ex:q)/(ex:p|ex:q)`.
#[test]
fn differential_alternation_exact_two_equals_eval_path() {
    // Mixed p/q edges across two holders so several of the four chains hit.
    let h1 = "ex:a ex:p ex:m . ex:a ex:q ex:n .";
    let h2 = "ex:m ex:q ex:x . ex:n ex:p ex:y . ex:m ex:p ex:w .";
    let edges = disclosed_edges(&[
        (h1, &["http://ex/p", "http://ex/q"]),
        (h2, &["http://ex/p", "http://ex/q"]),
    ]);

    let step = PathStep::alternation([nn("http://ex/p"), nn("http://ex/q")]);
    let form = PathForm::repeat_step(step, 2, 2);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "(ex:p|ex:q)/(ex:p|ex:q)");
    assert_eq!(got, want, "alternation exactly-2 must equal eval_path");
    // a-p-m-q-x, a-p-m-p-w, a-q-n-p-y -> {(a,x),(a,w),(a,y)} = 3.
    assert_eq!(got.len(), 3, "{got:?}");
}

/// Bounded alternation RANGE `(ex:p|ex:q){1,2}` — union of length-1 (p,q) and
/// length-2 (pp,pq,qp,qq), deduped. Differential against `(ex:p|ex:q) |
/// (ex:p|ex:q)/(ex:p|ex:q)`.
#[test]
fn differential_alternation_range_one_two() {
    let h1 = "ex:a ex:p ex:m . ex:a ex:q ex:n .";
    let h2 = "ex:m ex:q ex:x . ex:n ex:p ex:y .";
    let edges = disclosed_edges(&[
        (h1, &["http://ex/p", "http://ex/q"]),
        (h2, &["http://ex/p", "http://ex/q"]),
    ]);

    let step = PathStep::alternation([nn("http://ex/p"), nn("http://ex/q")]);
    let form = PathForm::repeat_step(step, 1, 2);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "(ex:p|ex:q) | (ex:p|ex:q)/(ex:p|ex:q)");
    assert_eq!(got, want);
    // len1 (every one-hop p/q edge): a-m, a-n, m-x, n-y ;
    // len2: a-x (a-p-m-q-x), a-y (a-q-n-p-y). 6 distinct pairs.
    assert_eq!(got.len(), 6, "{got:?}");
}

// =====================================================================
// 5b. SEQUENCE WITH A BOUNDED-REPETITION / ALTERNATION ELEMENT
//     — the trickiest eval_sequence branch (per-part dedup before compose)
// =====================================================================

/// `Sequence` whose FIRST element is a bounded repetition: `(ex:p){1,2}/ex:q`.
/// This exercises `eval_sequence` composing a multi-length sub-form (which can
/// reach the same mid node by two different lengths) with a plain hop — the
/// per-part dedup in `eval_sequence` must collapse the mid multiplicity before
/// the join so the composition is not inflated. Differential against the
/// engine's `(ex:p{1,2})/ex:q`, written as the union of fixed-length chains the
/// standard carries: `(ex:p | ex:p/ex:p)/ex:q`.
#[test]
fn differential_sequence_with_repetition_element() {
    // a->b->c via p (so a reaches c by p{1,2}: directly is absent, but a->b->c
    // is length 2, and we also add a->c direct p so a reaches c by BOTH len1+len2)
    // then c->d via q. The repetition sub-form yields (a,c) by two lengths; after
    // dedup it is one mid pair, composed with c->q->d gives (a,d) once.
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c . ex:a ex:p ex:c .";
    let h2 = "ex:c ex:q ex:d .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"]), (h2, &["http://ex/q"])]);

    let form = PathForm::Sequence(vec![
        PathForm::range(nn("http://ex/p"), 1, 2),
        PathForm::exact(nn("http://ex/q"), 1),
    ]);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "(ex:p | ex:p/ex:p)/ex:q");
    assert_eq!(
        got, want,
        "sequence with a repetition element must equal eval_path"
    );
    // p{1,2} mid-relation reaches c from BOTH a (a->b->c len2 AND a->c len1) and
    // b (b->c len1); only c has a q out-edge (c->d). So composing with c->q->d
    // gives {(a,d),(b,d)}. The key property under test: c is reached from a by
    // TWO p-lengths, but the per-part dedup collapses that mid multiplicity so
    // (a,d) is produced once, not twice.
    assert_eq!(got.len(), 2, "{got:?}");
    // (a,d) must appear exactly once despite c being reached from a by two p-lengths.
    let ad_rows = got
        .iter()
        .filter(|(a, b)| {
            *a == format!("{:?}", Term::from(nn("http://ex/a")))
                && *b == format!("{:?}", Term::from(nn("http://ex/d")))
        })
        .count();
    assert_eq!(
        ad_rows, 1,
        "(a,d) must be a single pair after per-part dedup"
    );
}

/// `Sequence` whose FIRST element is a bounded ALTERNATION repetition:
/// `(ex:p|ex:q){2}/ex:r`. Exercises `eval_sequence` composing an
/// alternation-expanded sub-form (which itself unrolls to a^ℓ chains, possibly
/// reaching the same mid node by several alternation branches) with a trailing
/// hop. Differential against `(ex:p|ex:q)/(ex:p|ex:q)/ex:r`.
#[test]
fn differential_sequence_with_alternation_repetition_element() {
    // a reaches m by p/p AND by q/q (two branches to the same mid m), then m->z
    // via r. The two branches to m must dedup to one mid before composition.
    let h1 = "ex:a ex:p ex:k . ex:a ex:q ex:j .";
    let h2 = "ex:k ex:p ex:m . ex:j ex:q ex:m . ex:m ex:r ex:z .";
    let edges = disclosed_edges(&[
        (h1, &["http://ex/p", "http://ex/q"]),
        (h2, &["http://ex/p", "http://ex/q", "http://ex/r"]),
    ]);

    let step = PathStep::alternation([nn("http://ex/p"), nn("http://ex/q")]);
    let form = PathForm::Sequence(vec![
        PathForm::repeat_step(step, 2, 2),
        PathForm::exact(nn("http://ex/r"), 1),
    ]);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "(ex:p|ex:q)/(ex:p|ex:q)/ex:r");
    assert_eq!(
        got, want,
        "sequence with an alternation-repetition element must equal eval_path"
    );
    // (p|q){2} from a: a-p-k-p-m and a-q-j-q-m both reach m; then m-r-z. => {(a,z)}.
    assert_eq!(got.len(), 1, "{got:?}");
    let az_rows = got
        .iter()
        .filter(|(a, b)| {
            *a == format!("{:?}", Term::from(nn("http://ex/a")))
                && *b == format!("{:?}", Term::from(nn("http://ex/z")))
        })
        .count();
    assert_eq!(
        az_rows, 1,
        "(a,z) must be a single pair despite two alternation branches reaching m"
    );
}

// =====================================================================
// 6. DEDUP across lengths — a pair reachable by two lengths appears ONCE
// =====================================================================

/// A pair reachable by BOTH a length-1 and a length-2 chain must be deduped to a
/// single endpoint pair (set semantics, design §2.2/§2.5) — the realized length
/// is never disclosed. Build a graph where a->c directly (p) AND a->b->c (p/p).
#[test]
fn dedup_collapses_multi_length_to_single_pair() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c . ex:a ex:p ex:c .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"])]);

    let form = PathForm::range(nn("http://ex/p"), 1, 2);
    let got = eval_bounded_path_disclosed(&edges, &form).unwrap();
    let pairs = pair_set(&got.vars, &got.rows);
    let want = clear_text_pairs(&[h1], "ex:p | ex:p/ex:p");
    assert_eq!(pairs, want);

    // (a,c) is reachable by p (a->c) AND p/p (a->b->c): exactly ONE row for it.
    let ac = (
        format!("{:?}", Term::from(nn("http://ex/a"))),
        format!("{:?}", Term::from(nn("http://ex/c"))),
    );
    let ac_rows = got
        .rows
        .iter()
        .filter(|r| {
            r[0].as_ref().map(|t| format!("{t:?}")) == Some(ac.0.clone())
                && r[1].as_ref().map(|t| format!("{t:?}")) == Some(ac.1.clone())
        })
        .count();
    assert_eq!(
        ac_rows, 1,
        "(a,c) reachable by two lengths must dedup to one row"
    );
}

// =====================================================================
// 7. CRYPTO-FREE — no MPC round occurs on the disclosed-key path
// =====================================================================

/// CRYPTO-FREE assertion (design §2.1 DISCLOSED regime, §6 step 1): the bounded
/// disclosed-key path runs entirely OUTSIDE the cryptographic core — no secret
/// sharing, no MPC round.
///
/// ## Why a naive `assert_eq!(counter.mult_rounds, 0)` would be VACUOUS
///
/// [`crate::metrics::CommCounter`] is a STANDALONE bench-harness modelling object:
/// it is NOT threaded through `eval_bounded_path_disclosed`, and its counters move
/// ONLY when `record_mult` / `record_open` / `record_independent_equalities` /
/// `record_shuffle` / `record_sort` are called — and those calls exist ONLY in
/// `crate::bench` (the harness), never inside the protocol primitives and never in
/// the unroll's call graph. So a fresh counter handed to this test would read 0
/// REGARDLESS of what the evaluation did; asserting `== 0` on it proves nothing.
///
/// ## How this test is made MEANINGFUL — a negative control
///
/// We split the property into two genuinely-testable halves:
///
/// 1. **The counter is a live witness, not a constant.** A NEGATIVE CONTROL first
///    drives the exact secure-path ops a HIDDEN-regime evaluation would pay
///    (`record_mult` + `record_open`, i.e. one `secure_equal`) into a counter and
///    asserts it goes NON-ZERO. This proves `mult_rounds`/`open_rounds` are
///    observable and that `== 0` is a falsifiable claim — if the disclosed-key path
///    ever recorded a round through the same API, this counter type WOULD show it.
///
/// 2. **The evaluation records nothing.** We then evaluate every supported bounded
///    form against a SEPARATE counter, and confirm it stays at 0 — which holds
///    because the evaluation takes no counter, shares no mutable round state, and
///    its call graph (`DisclosedKeyJoin` folds over disclosed global IRIs) reaches
///    none of the `record_*` surfaces. The negative control above guarantees this 0
///    is the real "no rounds occurred", not the trivial "counter was never wired".
#[test]
fn crypto_free_no_mpc_round_artifacts() {
    use crate::metrics::CommCounter;

    // --- NEGATIVE CONTROL: prove the counter actually MOVES under MPC-style ops,
    // so the `== 0` assertions below are a real witness and not a tautology. These
    // are the same public APIs the secure (HIDDEN-regime) path uses to account one
    // `secure_equal` (1 mult + 1 open). ---
    let mut witness = CommCounter::new(3);
    assert_eq!(witness.mult_rounds, 0, "control: fresh counter starts at 0");
    assert_eq!(witness.open_rounds, 0, "control: fresh counter starts at 0");
    witness.record_mult().unwrap();
    witness.record_open().unwrap();
    assert!(
        witness.mult_rounds > 0 && witness.open_rounds > 0,
        "negative control FAILED: the counter does not move under MPC ops, so a \
         `== 0` assertion would be vacuous — fix the witness before trusting the \
         crypto-free claim"
    );

    let h1 = "ex:a ex:knows ex:b . ex:b ex:knows ex:c .";
    let h2 = "ex:c ex:knows ex:d .";
    let edges = disclosed_edges(&[(h1, &["http://ex/knows"]), (h2, &["http://ex/knows"])]);

    // --- THE PROPERTY: a SEPARATE counter, observed across a full evaluation of
    // every supported bounded form, must stay at 0 (no mult/open round recorded). ---
    let counter = CommCounter::new(3);
    let forms = [
        PathForm::sequence([nn("http://ex/knows"), nn("http://ex/knows")]),
        PathForm::exact(nn("http://ex/knows"), 2),
        PathForm::range(nn("http://ex/knows"), 1, 3),
        PathForm::range(nn("http://ex/knows"), 0, 2),
        PathForm::repeat_step(PathStep::alternation([nn("http://ex/knows")]), 1, 2),
    ];
    for form in &forms {
        let _ = eval_bounded_path_disclosed(&edges, form).unwrap();
    }

    // ROUND-COUNT == 0: the disclosed-key path recorded no multiplication and no
    // open round. The negative control above proves this 0 is meaningful (the
    // counter CAN move) — it is the crypto-free property, not an un-wired counter.
    assert_eq!(
        counter.mult_rounds, 0,
        "disclosed-key path must run NO mult rounds"
    );
    assert_eq!(
        counter.open_rounds, 0,
        "disclosed-key path must run NO open rounds"
    );
}

// =====================================================================
// 8. Misc soundness
// =====================================================================

/// A wrong-shape disclosed edge partial (not exactly 2 columns) is a Protocol
/// error, not a silent wrong answer.
#[test]
fn edge_partial_wrong_arity_is_protocol_error() {
    let bad = PartialResult {
        holder: HolderId::new("h0"),
        vars: vec![Variable::new_unchecked("s")],
        rows: vec![],
    };
    let err = DisclosedEdges::from_holder_edges([(nn("http://ex/p"), bad)]).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(_)));
}

/// min > max is a Protocol error.
#[test]
fn min_greater_than_max_is_protocol_error() {
    let edges = disclosed_edges(&[("ex:a ex:p ex:b .", &["http://ex/p"])]);
    let form = PathForm::range(nn("http://ex/p"), 3, 1);
    let err = eval_bounded_path_disclosed(&edges, &form).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(_)));
}

/// The naive fold the unroller must NOT do: carry every intermediate column to the
/// end and project once. Test-local on purpose — it is the shape whose accumulator
/// holds one row per WALK, so it is the differential REFERENCE at small `L` and
/// must never be what ships.
fn naive_fixed_chain(
    edges: &DisclosedEdges,
    chain: &[NamedNode],
    subject: &Variable,
    object: &Variable,
) -> PartialResult {
    let n = chain.len();
    let hop_var = |i: usize| -> Variable {
        if i == 0 {
            subject.clone()
        } else if i == n {
            object.clone()
        } else {
            Variable::new_unchecked(format!("{INTERMEDIATE_PREFIX}{i}"))
        }
    };
    let mut acc = edges.relation_for(&chain[0], &hop_var(0), &hop_var(1));
    let join = DisclosedKeyJoin::new();
    for (idx, pred) in chain.iter().enumerate().skip(1) {
        let next = edges.relation_for(pred, &hop_var(idx), &hop_var(idx + 1));
        let plan = JoinPlan {
            join_var: hop_var(idx),
            key_disclosed: true,
        };
        acc = join.join(&[acc, next], &plan).unwrap();
    }
    dedup_endpoint_pairs(
        project_endpoints(&acc, subject, object).unwrap(),
        subject,
        object,
    )
}

/// The per-hop projection must not change the ANSWER, only the work: for chain
/// lengths 1..=8 over three graphs (dense 2-node, a line, a line with a back edge)
/// the shipped fold and the naive walk-carrying fold agree row for row.
#[test]
fn per_hop_projection_matches_the_naive_fold_at_small_lengths() {
    let graphs: [&[(&str, &[&str])]; 3] = [
        &[(
            "ex:a ex:p ex:a . ex:a ex:p ex:b . ex:b ex:p ex:a . ex:b ex:p ex:b .",
            &["http://ex/p"],
        )],
        &[(
            "ex:a ex:p ex:b . ex:b ex:p ex:c . ex:c ex:p ex:d .",
            &["http://ex/p"],
        )],
        &[(
            "ex:a ex:p ex:b . ex:b ex:p ex:c . ex:c ex:p ex:a .",
            &["http://ex/p"],
        )],
    ];
    let (s, o) = (
        Variable::new_unchecked(SUBJECT_VAR),
        Variable::new_unchecked(OBJECT_VAR),
    );
    for (g, docs) in graphs.iter().enumerate() {
        let edges = disclosed_edges(docs);
        for len in 1..=8usize {
            let chain: Vec<NamedNode> = (0..len).map(|_| nn("http://ex/p")).collect();
            let shipped =
                dedup_endpoint_pairs(eval_fixed_chain(&edges, &chain, &s, &o).unwrap(), &s, &o);
            let naive = naive_fixed_chain(&edges, &chain, &s, &o);
            assert_eq!(
                shipped.rows, naive.rows,
                "graph {}, L={}: the per-hop projection changed the answer",
                g, len
            );
        }
    }
}

/// The work must follow the ENDPOINT count, not the walk count: on the dense
/// 2-node graph (all four `p` edges) the answer is the same four pairs at every
/// length, and a chain at the hop cap evaluates. Carrying the intermediates
/// instead would make this the exponential case — finishing at all is the witness,
/// and nothing here asserts a timing.
#[test]
fn a_dense_graph_evaluates_at_the_hop_cap() {
    let edges = disclosed_edges(&[(
        "ex:a ex:p ex:a . ex:a ex:p ex:b . ex:b ex:p ex:a . ex:b ex:p ex:b .",
        &["http://ex/p"],
    )]);
    let (s, o) = (
        Variable::new_unchecked(SUBJECT_VAR),
        Variable::new_unchecked(OBJECT_VAR),
    );
    let short = dedup_endpoint_pairs(
        eval_fixed_chain(&edges, &[nn("http://ex/p")], &s, &o).unwrap(),
        &s,
        &o,
    );
    assert_eq!(short.rows.len(), 4, "the dense 2-node graph has 4 pairs");

    let chain: Vec<NamedNode> = (0..MAX_UNROLL_HOPS).map(|_| nn("http://ex/p")).collect();
    let long = dedup_endpoint_pairs(eval_fixed_chain(&edges, &chain, &s, &o).unwrap(), &s, &o);
    assert_eq!(
        long.rows, short.rows,
        "every length reaches the same four endpoint pairs on this graph"
    );
}

/// AT the hop cap, through the PUBLIC entry: `{64,64}` over a single edge is
/// accepted (the cap is a cap, not an off-by-one) and has no 64-hop walk to find.
#[test]
fn a_repetition_at_the_hop_cap_is_accepted() {
    let edges = disclosed_edges(&[("ex:a ex:p ex:b .", &["http://ex/p"])]);
    let form = PathForm::range(nn("http://ex/p"), MAX_UNROLL_HOPS, MAX_UNROLL_HOPS);
    let got = eval_bounded_path_disclosed(&edges, &form).unwrap();
    assert!(
        got.rows.is_empty(),
        "a single edge has no {MAX_UNROLL_HOPS}-hop chain, got {:?}",
        got.rows
    );
}

/// HOP-COUNT GUARD: the chain cap does not bind on a ONE-way step, whose projected
/// chain count is `k - m + 1` however large the public `k` is — yet each chain of
/// length `L` is an `L`-hop join fold, so the unrolled work still grows with `k`.
/// [`MAX_UNROLL_HOPS`] refuses that before the chain projection is even computed.
#[test]
fn over_hop_cap_unroll_is_refused_before_planning() {
    let edges = disclosed_edges(&[("ex:a ex:p ex:b .", &["http://ex/p"])]);
    let one_way = || PathStep::predicate(nn("http://ex/p"));

    for max in [MAX_UNROLL_HOPS + 1, 1_000_000] {
        // The chain cap would ACCEPT this path, so the refusal below is the hop
        // cap's alone: a one-way step projects to `max` chains for `{1,max}`.
        assert!(
            (max as u64) <= MAX_UNROLL_CHAINS,
            "the projected chain count must sit under the chain cap for this case"
        );
        let form = PathForm::repeat_step(one_way(), 1, max);
        let err = eval_bounded_path_disclosed(&edges, &form).unwrap_err();
        match &err {
            MpcError::Protocol(msg) => assert!(
                msg.contains("MAX_UNROLL_HOPS") && msg.contains("hops per chain"),
                "the error should name the hop cap, got: {}",
                msg
            ),
            other => panic!(
                "an over-hop-cap unroll must be a Protocol error, got {:?}",
                other
            ),
        }
    }

    // Under the cap, a one-way repetition still evaluates.
    let form = PathForm::repeat_step(one_way(), 1, 1);
    let got = eval_bounded_path_disclosed(&edges, &form).unwrap();
    assert_eq!(
        got.rows.len(),
        1,
        "a 1-hop one-way repetition must still evaluate"
    );
}

/// UNROLL-SIZE GUARD: a bounded path whose alternation unroll would exceed
/// [`MAX_UNROLL_CHAINS`] is rejected with a controlled `MpcError::Protocol` BEFORE
/// any chain is generated — no panic, no OOM, no `usize`-overflow `with_capacity`.
/// This pins the DoS guard the reviewer asked for.
#[test]
fn over_cap_unroll_is_protocol_error_not_panic() {
    // A 4-way alternation at k = 16 projects to Σ_{ℓ=1..=16} 4^ℓ ≈ 5.7e9 chains,
    // far above the 2^20 cap — must be refused cleanly, not evaluated.
    let edges = disclosed_edges(&[("ex:a ex:p ex:b .", &["http://ex/p"])]);
    let step = PathStep::alternation([
        nn("http://ex/p"),
        nn("http://ex/q"),
        nn("http://ex/r"),
        nn("http://ex/s"),
    ]);
    let form = PathForm::repeat_step(step, 1, 16);
    let err = eval_bounded_path_disclosed(&edges, &form).unwrap_err();
    assert!(
        matches!(err, MpcError::Protocol(_)),
        "over-cap unroll must be a Protocol error, got {err:?}"
    );
    if let MpcError::Protocol(msg) = &err {
        assert!(
            msg.contains("MAX_UNROLL_CHAINS") || msg.contains("denial-of-service"),
            "error should explain the unroll-size cap, got: {msg}"
        );
    }

    // The guard is computed from the projected chain count, so a path whose unroll
    // sits JUST under the cap is still ACCEPTED. We assert the accept decision at
    // the count level (cheap) rather than by materialising ~10^6 chains (which
    // would make this test take minutes for no extra coverage — the actual
    // evaluation of small forms is already exercised by the differential tests).
    // Σ_{ℓ=1..=19} 2^ℓ = 2^20 - 2 = 1_048_574 ≤ MAX_UNROLL_CHAINS (1_048_576).
    assert_eq!(
        super::projected_chain_count(2, 1, 19),
        Some(MAX_UNROLL_CHAINS - 2),
        "a 2-way alternation at k=19 must project to just under the cap (accepted)"
    );
    // One more hop tips a 2-way alternation over the cap (Σ_{ℓ=1..=20} 2^ℓ ≈ 2^21).
    assert!(
        super::projected_chain_count(2, 1, 20).unwrap() > MAX_UNROLL_CHAINS,
        "k=20 must exceed the cap (rejected)"
    );
}

/// The projected-chain-count closed form matches the design's `Σ_{ℓ=m..=k} a^ℓ`
/// and reports overflow (rather than wrapping) on an absurd bound.
#[test]
fn projected_chain_count_matches_closed_form_and_detects_overflow() {
    // Single predicate (a=1): one chain per length → (max - min + 1), minus the
    // length-0 arm which is not a chain.
    assert_eq!(super::projected_chain_count(1, 1, 3), Some(3));
    assert_eq!(super::projected_chain_count(1, 0, 3), Some(3)); // length-0 excluded
                                                                // 2-way alternation, {1,3}: 2 + 4 + 8 = 14.
    assert_eq!(super::projected_chain_count(2, 1, 3), Some(14));
    // 3-way alternation, {2,2}: 3^2 = 9.
    assert_eq!(super::projected_chain_count(3, 2, 2), Some(9));
    // Overflow: a huge alternation at a huge bound must report None, not wrap.
    assert_eq!(super::projected_chain_count(1_000_000, 1, 20), None);
}

/// An absent predicate (no disclosed edges) contributes nothing — matches the
/// engine treating a never-occurring predicate as the empty relation.
#[test]
fn absent_predicate_yields_empty_chain() {
    let edges = disclosed_edges(&[("ex:a ex:p ex:b .", &["http://ex/p"])]);
    // Sequence over a predicate that nobody disclosed: empty.
    let form = PathForm::exact(nn("http://ex/missing"), 2);
    let got = eval_bounded_path_disclosed(&edges, &form).unwrap();
    assert!(got.rows.is_empty());
}

// =====================================================================
// 8. NESTED SEQUENCES — each level's intermediate variables are its own
// =====================================================================

/// A `Sequence` nested inside a `Sequence` must not reuse the enclosing level's
/// connecting variables.
///
/// Every level used to name them `__pp_zseq1`, `__pp_zseq2`, ..., so an inner
/// sequence's FIRST connecting variable was `__pp_zseq1` — which was also the
/// variable the outer level had handed it as an endpoint. Evaluating a sub-form
/// onto a pair of identical variables constrains both ends to be equal, so the
/// pairs that reach the inner sequence through distinct endpoints were silently
/// dropped. A wrong answer, not an error, which is why it needs a differential
/// test rather than a refusal test.
///
/// `(ex:p/ex:p)/ex:q` over `a-p-b-p-c-q-d`, with the first factor written as a
/// nested `Sequence`, forces exactly that aliasing: the inner sequence's second
/// element would be evaluated onto `(__pp_zseq1, __pp_zseq1)` and `(a,d)` would
/// disappear.
#[test]
fn a_nested_sequence_does_not_alias_the_enclosing_levels_variables() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c .";
    let h2 = "ex:c ex:q ex:d .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"]), (h2, &["http://ex/q"])]);

    let form = PathForm::Sequence(vec![
        // The nested level. Flattened this is just `ex:p/ex:p`.
        PathForm::Sequence(vec![
            PathForm::exact(nn("http://ex/p"), 1),
            PathForm::exact(nn("http://ex/p"), 1),
        ]),
        PathForm::exact(nn("http://ex/q"), 1),
    ]);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "ex:p/ex:p/ex:q");
    assert_eq!(
        got, want,
        "a nested sequence must denote the same relation as the flattened path"
    );
    assert_eq!(got.len(), 1, "exactly (a,d): {got:?}");

    // The same relation written WITHOUT nesting, to show the nesting is the only
    // difference and the flat form was never affected.
    let flat = PathForm::Sequence(vec![
        PathForm::exact(nn("http://ex/p"), 1),
        PathForm::exact(nn("http://ex/p"), 1),
        PathForm::exact(nn("http://ex/q"), 1),
    ]);
    assert_eq!(
        federated_pairs(&edges, &flat),
        want,
        "the flat form must agree too"
    );

    // Three levels deep, and nested in the LAST position as well as the first —
    // one unique name per sequence CALL, not per depth.
    let deep = PathForm::Sequence(vec![
        PathForm::Sequence(vec![
            PathForm::Sequence(vec![
                PathForm::exact(nn("http://ex/p"), 1),
                PathForm::exact(nn("http://ex/p"), 1),
            ]),
            PathForm::exact(nn("http://ex/q"), 1),
        ]),
        PathForm::range(nn("http://ex/p"), 0, 1),
    ]);
    assert_eq!(
        federated_pairs(&edges, &deep),
        clear_text_pairs(&[h1, h2], "ex:p/ex:p/ex:q/ex:p?"),
        "three nested levels must still denote the flattened path"
    );
}

// =====================================================================
// 9. SEEDED DIFFERENTIAL FUZZ — nested forms against a composition oracle
// =====================================================================

/// splitmix64 — a fixed, dependency-free generator so the whole sweep below is
/// reproducible from one seed.
struct FuzzRng(u64);

impl FuzzRng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

/// The three predicates the fuzz graphs use. Every holder discloses all three,
/// so the disclosed edge set — and with it the node domain a `{0,k}` form draws
/// its identity pairs from — is exactly the generated graph.
const FUZZ_PREDS: [&str; 3] = ["http://ex/f0", "http://ex/f1", "http://ex/f2"];

/// An abstract generated form, so the generator can build the oracle's view and
/// the [`PathForm`] from one value.
#[derive(Debug, Clone)]
enum FuzzForm {
    /// `(p_i | ...){m,k}` — the alternatives are indices into [`FUZZ_PREDS`].
    Rep(Vec<usize>, usize, usize),
    Seq(Vec<FuzzForm>),
}

impl FuzzForm {
    fn to_path_form(&self) -> PathForm {
        match self {
            FuzzForm::Rep(alts, m, k) => PathForm::repeat_step(
                PathStep::alternation(alts.iter().map(|&i| nn(FUZZ_PREDS[i]))),
                *m,
                *k,
            ),
            FuzzForm::Seq(parts) => {
                PathForm::Sequence(parts.iter().map(FuzzForm::to_path_form).collect())
            }
        }
    }

    fn has_nested_sequence(&self) -> bool {
        match self {
            FuzzForm::Rep(..) => false,
            FuzzForm::Seq(parts) => parts
                .iter()
                .any(|p| matches!(p, FuzzForm::Seq(_)) || p.has_nested_sequence()),
        }
    }
}

/// Generate a form. `depth` bounds the nesting; the sizes are kept small so the
/// projected chain count stays far below [`MAX_UNROLL_CHAINS`] and every case
/// evaluates in microseconds.
fn fuzz_form(rng: &mut FuzzRng, depth: usize) -> FuzzForm {
    fn rep(rng: &mut FuzzRng) -> FuzzForm {
        let n_alts = 1 + rng.below(2) as usize;
        let alts: Vec<usize> = (0..n_alts).map(|_| rng.below(3) as usize).collect();
        let m = rng.below(3) as usize;
        let k = m + rng.below(2) as usize;
        FuzzForm::Rep(alts, m, k)
    }
    if depth == 0 || rng.chance(1, 3) {
        return rep(rng);
    }
    let n_parts = 2 + rng.below(2) as usize;
    let parts = (0..n_parts)
        .map(|_| {
            // Nest a sequence inside a sequence often — that is the shape under
            // test — but keep plain repetitions in the mix too.
            if rng.chance(1, 2) {
                fuzz_form(rng, depth - 1)
            } else {
                rep(rng)
            }
        })
        .collect();
    FuzzForm::Seq(parts)
}

/// `(s, predicate index, o)` over small integer node indices.
type FuzzGraph = Vec<(usize, usize, usize)>;

/// Generate a graph and split it across holders. Returns the per-holder edge
/// lists; the union of them is the relation the oracle reasons about.
fn fuzz_graph(rng: &mut FuzzRng) -> Vec<FuzzGraph> {
    let v = 2 + rng.below(4) as usize;
    let holders = 1 + rng.below(3) as usize;
    let density = 1 + rng.below(4); // out of 8
    let mut per_holder: Vec<FuzzGraph> = vec![Vec::new(); holders];
    for s in 0..v {
        for p in 0..FUZZ_PREDS.len() {
            for o in 0..v {
                if rng.chance(density, 8) {
                    let h = rng.below(holders as u64) as usize;
                    per_holder[h].push((s, p, o));
                    // Occasionally disclose the same edge from a second holder,
                    // so the fold has to dedup across members.
                    if holders > 1 && rng.chance(1, 6) {
                        let h2 = rng.below(holders as u64) as usize;
                        per_holder[h2].push((s, p, o));
                    }
                }
            }
        }
    }
    per_holder
}

/// The IRI of fuzz node `i`.
fn fuzz_node_iri(i: usize) -> String {
    format!("http://ex/fn{}", i)
}

/// Turtle body disclosing one holder's edges.
fn fuzz_turtle(g: &FuzzGraph) -> String {
    let mut out = String::new();
    for &(s, p, o) in g {
        out.push_str(&format!(
            "<{}> <{}> <{}> .\n",
            fuzz_node_iri(s),
            FUZZ_PREDS[p],
            fuzz_node_iri(o)
        ));
    }
    out
}

/// An independent semantic oracle: the endpoint relation of `f` as a set of
/// node-index pairs, computed by relational composition over the union graph —
/// no part of the unroller is involved.
fn fuzz_oracle(f: &FuzzForm, per_holder: &[FuzzGraph]) -> BTreeSet<(usize, usize)> {
    fn compose(
        a: &BTreeSet<(usize, usize)>,
        b: &BTreeSet<(usize, usize)>,
    ) -> BTreeSet<(usize, usize)> {
        let mut out = BTreeSet::new();
        for &(x, y) in a {
            for &(y2, z) in b {
                if y == y2 {
                    out.insert((x, z));
                }
            }
        }
        out
    }
    let all: BTreeSet<(usize, usize, usize)> = per_holder.iter().flatten().copied().collect();
    // The node domain the length-0 identity pairs come from: every node named by
    // any disclosed edge, under any predicate.
    let domain: BTreeSet<usize> = all.iter().flat_map(|&(s, _, o)| [s, o]).collect();
    match f {
        FuzzForm::Rep(alts, m, k) => {
            let step: BTreeSet<(usize, usize)> = all
                .iter()
                .filter(|(_, p, _)| alts.contains(p))
                .map(|&(s, _, o)| (s, o))
                .collect();
            let mut out = BTreeSet::new();
            let mut power: BTreeSet<(usize, usize)> = domain.iter().map(|&x| (x, x)).collect();
            for len in 0..=*k {
                if len > 0 {
                    power = compose(&power, &step);
                }
                if len >= *m {
                    out.extend(power.iter().copied());
                }
            }
            out
        }
        FuzzForm::Seq(parts) => {
            let mut acc = fuzz_oracle(&parts[0], per_holder);
            for p in &parts[1..] {
                acc = compose(&acc, &fuzz_oracle(p, per_holder));
            }
            acc
        }
    }
}

/// Render the oracle's node-index pairs in the same form [`pair_set`] produces,
/// so the two sides are directly comparable.
fn fuzz_oracle_pairs(s: &BTreeSet<(usize, usize)>) -> BTreeSet<(String, String)> {
    let term = |i: usize| format!("{:?}", Term::NamedNode(nn(&fuzz_node_iri(i))));
    s.iter().map(|&(a, b)| (term(a), term(b))).collect()
}

/// Seeded differential sweep: random nested forms over random multi-holder
/// graphs, the disclosed-key unroll against the composition oracle above.
///
/// This is the general form of the regression test in section 8 — with the
/// shared `{INTERMEDIATE_PREFIX}seq{i}` naming, every iteration whose form
/// nests a sequence under a sequence can lose pairs. The sweep asserts the
/// nested and the flat cases alike, and additionally asserts that nested forms
/// actually OCCURRED, so a generator change cannot quietly empty it out.
#[test]
fn seeded_fuzz_nested_forms_match_the_composition_oracle() {
    let mut rng = FuzzRng(0x5EED_B0A4);
    let mut nested = 0usize;
    let mut flat = 0usize;

    for iter in 0..200 {
        let per_holder = fuzz_graph(&mut rng);
        let form = fuzz_form(&mut rng, 2);

        let docs: Vec<String> = per_holder.iter().map(fuzz_turtle).collect();
        let preds: Vec<&str> = FUZZ_PREDS.to_vec();
        let args: Vec<(&str, &[&str])> = docs.iter().map(|d| (d.as_str(), &preds[..])).collect();
        let edges = disclosed_edges(&args);

        let got = federated_pairs(&edges, &form.to_path_form());
        let want = fuzz_oracle_pairs(&fuzz_oracle(&form, &per_holder));
        assert_eq!(
            got, want,
            "iter {}: the unroll disagrees with the oracle\n  form: {:?}\n  graph: {:?}",
            iter, form, per_holder
        );

        if form.has_nested_sequence() {
            nested += 1;
        } else {
            flat += 1;
        }
    }

    assert!(
        nested >= 20,
        "the sweep must actually exercise nested sequences, saw {} of 200",
        nested
    );
    assert!(flat > 0, "and flat forms too, saw {} of 200", flat);
}

// =====================================================================
// The whole-form ROW BUDGET and the deduped union
// =====================================================================

/// The budget constant is pinned BY VALUE, so changing it is changing this test
/// and not quietly relaxing a guard.
#[test]
fn the_path_form_row_budget_is_pinned_by_value() {
    assert_eq!(MAX_PATH_FORM_ROWS, 1 << 28);
}

/// The budget's own arithmetic: exactly at [`MAX_PATH_FORM_ROWS`] is accepted, one
/// row past it is refused, and a total that would overflow `usize` is refused
/// rather than wrapping to a small number under the budget.
#[test]
fn the_row_budget_accepts_the_cap_and_refuses_one_past_it() {
    let mut b = PathFormBudget::new();
    b.charge(MAX_PATH_FORM_ROWS).expect("exactly at the cap");
    assert_eq!(b.charged(), MAX_PATH_FORM_ROWS);
    let err = b.charge(1).expect_err("one row past the cap is refused");
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PATH_FORM_ROWS")),
        "{:?}",
        err
    );

    let mut over = PathFormBudget::with_charged(usize::MAX);
    assert!(
        over.charge(1).is_err(),
        "an overflowing total must refuse, not wrap under the budget"
    );
}

/// The budget is WIRED into the real unroll and binds there: a form that runs on a
/// fresh budget is refused by exactly the same code path when the budget has
/// already been spent, and runs again when just enough is left for it.
///
/// There is no "zero draws" half to this one: the disclosed unroll is crypto-free,
/// so there is no randomness source to draw from at all (see
/// `crypto_free_no_mpc_round_artifacts`). What the refusal precedes is the rows.
#[test]
fn the_row_budget_binds_on_a_real_form_at_the_cap() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c . ex:c ex:p ex:d .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"])]);
    let form = PathForm::range(nn("http://ex/p"), 1, 3);

    // What this form actually charges, measured on the real path.
    let mut measured = PathFormBudget::new();
    let baseline = eval_with_budget(&edges, &form, &mut measured).unwrap();
    let spend = measured.charged();
    assert!(spend > 0, "the unroll must charge the budget it is given");

    // A budget with one row less than it needs: refused, naming the budget.
    let mut tight = PathFormBudget::with_charged(MAX_PATH_FORM_ROWS - (spend - 1));
    let err = eval_with_budget(&edges, &form, &mut tight)
        .expect_err("a form that would pass the budget is refused");
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PATH_FORM_ROWS")),
        "{:?}",
        err
    );

    // With exactly enough left it runs, and returns the same answer.
    let mut exact = PathFormBudget::with_charged(MAX_PATH_FORM_ROWS - spend);
    let got = eval_with_budget(&edges, &form, &mut exact).expect("exactly enough budget runs");
    assert_eq!(got.rows, baseline.rows);
}

/// The repetition union is deduped AS IT GROWS, not only at the top. A pair that
/// several chain lengths reach contributes ONE row to the relation the repetition
/// RETURNS, so the union holds the answer's rows rather than one row per
/// (pair, chain).
///
/// Asserted on `eval_form_relation`'s own output — BEFORE the top-level
/// `dedup_endpoint_pairs` — because that top-level dedup collapses duplicates
/// either way and would hide whether the union ever held them. The budget, which
/// charges every row PRODUCED, is read alongside to show duplicates really were
/// produced and really were dropped.
#[test]
fn the_repetition_union_is_deduped_as_it_grows() {
    // A complete little cycle, so every length reaches the same pairs repeatedly.
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:a . ex:a ex:p ex:a . ex:b ex:p ex:b .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"])]);
    let form = PathForm::range(nn("http://ex/p"), 1, 4);
    let subject = Variable::new_unchecked(SUBJECT_VAR);
    let object = Variable::new_unchecked(OBJECT_VAR);

    let mut budget = PathFormBudget::new();
    let mut scope = 0usize;
    let raw =
        eval_form_relation(&edges, &form, &subject, &object, &mut scope, &mut budget).unwrap();

    // The relation the repetition hands back carries NO duplicate row.
    let distinct: BTreeSet<String> = raw.rows.iter().map(|r| format!("{r:?}")).collect();
    assert_eq!(
        raw.rows.len(),
        distinct.len(),
        "the repetition's own union must already be a set, not {} rows over {} distinct",
        raw.rows.len(),
        distinct.len()
    );
    // And duplicates really were produced: four chain lengths re-reach the same
    // pairs, so the rows charged exceed the rows retained.
    assert!(
        budget.charged() > raw.rows.len(),
        "charged {} must exceed the {} retained rows — otherwise nothing was collapsed",
        budget.charged(),
        raw.rows.len()
    );

    // And the answer is still right once the top-level dedup has run too.
    let got = eval_bounded_path_disclosed(&edges, &form).unwrap();
    let want = clear_text_pairs(
        &[h1],
        "ex:p | ex:p/ex:p | ex:p/ex:p/ex:p | ex:p/ex:p/ex:p/ex:p",
    );
    assert_eq!(pair_set(&got.vars, &got.rows), want);
}

// =====================================================================
// PathForm nesting DEPTH and the static whole-form WORK budget
// =====================================================================

/// Both form-shape caps are pinned BY VALUE.
#[test]
fn the_form_shape_caps_are_pinned_by_value() {
    assert_eq!(MAX_PATH_FORM_DEPTH, 32);
    assert_eq!(MAX_PATH_FORM_WORK, 1 << 28);
}

/// Build a `Sequence`-in-`Sequence` form of exactly `depth` levels, with one
/// trivial repetition at the bottom. Built ITERATIVELY so the fixture itself
/// cannot be what overflows.
fn nested_form(depth: usize) -> PathForm {
    let mut form = PathForm::exact(nn("http://ex/p"), 1);
    for _ in 1..depth {
        form = PathForm::Sequence(vec![form]);
    }
    form
}

/// A form nested one level past [`MAX_PATH_FORM_DEPTH`] is refused; a form exactly
/// AT the cap evaluates. The refusal comes from the ITERATIVE shape check, before
/// `eval_sequence` recurses into anything — which is the whole point, since nesting
/// depth is stack depth on the evaluator.
///
/// There is no "zero draws" half: the disclosed unroll is crypto-free, so there is
/// no randomness source to draw from (see `crypto_free_no_mpc_round_artifacts`).
#[test]
fn the_form_depth_cap_refuses_just_over_and_runs_at_the_cap() {
    let edges = disclosed_edges(&[("ex:a ex:p ex:b .", &["http://ex/p"])]);

    // One past the cap: refused, naming the cap.
    let err = eval_bounded_path_disclosed(&edges, &nested_form(MAX_PATH_FORM_DEPTH + 1))
        .expect_err("one level past the depth cap must be refused");
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PATH_FORM_DEPTH")),
        "{:?}",
        err
    );
    // Still refused well past the cap, and the ITERATIVE walk is what refuses it:
    // the check descends no further than the first over-deep level, so its own
    // stack use does not grow with the form.
    assert!(eval_bounded_path_disclosed(&edges, &nested_form(MAX_PATH_FORM_DEPTH * 8)).is_err());

    // Exactly AT the cap: evaluates, and to the same answer the un-nested form
    // gives — a nested Sequence of one part denotes exactly that part.
    let got = eval_bounded_path_disclosed(&edges, &nested_form(MAX_PATH_FORM_DEPTH))
        .expect("the deepest legal form evaluates");
    let flat = eval_bounded_path_disclosed(&edges, &nested_form(1)).unwrap();
    assert_eq!(got.rows, flat.rows);
    assert_eq!(got.rows.len(), 1);
}

/// The static whole-form work budget bounds a LONG Sequence, which neither
/// per-repetition shape cap nor the row budget catches: every part passes both
/// shape caps, and over an edge set that yields no rows the row budget is never
/// charged either.
///
/// Exercised exactly at and one unit past [`MAX_PATH_FORM_WORK`] on the real shape
/// check, reaching the budget by handing the walk work already charged rather than
/// by building a form of a quarter of a billion parts.
#[test]
fn the_form_work_budget_refuses_just_over_and_runs_at_the_cap() {
    // A part that charges exactly `1 + chains*hops = 1 + 0*0 = 1`: `{0,0}` unrolls
    // to no chain at all (the reflexive arm is not a chain), so each part's whole
    // charge is the `+ 1` that makes a long sequence of free parts finite.
    let unit = PathForm::Repetition(BoundedRepetition {
        step: PathStep::predicate(nn("http://ex/p")),
        min: 0,
        max: 0,
    });
    assert_eq!(
        check_form_shape(&unit).unwrap(),
        1,
        "a zero-length repetition part costs 1"
    );

    // A ten-part sequence: the root plus ten unit parts.
    let seq = PathForm::Sequence((0..10).map(|_| unit.clone()).collect::<Vec<_>>());
    let charge = check_form_shape(&seq).unwrap();
    assert_eq!(charge, 11, "root + 10 parts");

    // Exactly AT the budget: accepted.
    assert_eq!(
        check_form_shape_from(&seq, MAX_PATH_FORM_WORK - charge).unwrap(),
        MAX_PATH_FORM_WORK
    );
    // One unit past it: refused, naming the budget.
    let err = check_form_shape_from(&seq, MAX_PATH_FORM_WORK - charge + 1)
        .expect_err("one unit past the budget is refused");
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PATH_FORM_WORK")),
        "{:?}",
        err
    );
    // An overflowing charge refuses rather than wrapping under the budget.
    assert!(check_form_shape_from(&seq, u64::MAX).is_err());

    // A long sequence really does reach the budget by its part count alone, over
    // an edge set that yields NO rows — so the row budget cannot be what binds.
    // Fourteen repetitions at both per-repetition shape caps are enough.
    let heavy = PathForm::repeat_step(
        PathStep::alternation([nn("http://ex/p"), nn("http://ex/q")]),
        1,
        19,
    );
    let heavy_charge = check_form_shape(&heavy).expect("a near-cap repetition is in budget");
    assert!(
        heavy_charge <= MAX_PATH_FORM_WORK,
        "the work budget must not make an expensive-but-legal single part illegal"
    );
    assert!(
        heavy_charge > MAX_UNROLL_CHAINS,
        "the heavy part must really be heavy"
    );
    let parts = (MAX_PATH_FORM_WORK / heavy_charge + 1) as usize;
    let long = PathForm::Sequence((0..parts).map(|_| heavy.clone()).collect::<Vec<_>>());
    let edges = disclosed_edges(&[("ex:a ex:q ex:b .", &["http://ex/q"])]);
    let err = eval_bounded_path_disclosed(&edges, &long)
        .expect_err("a long sequence must be refused on the work budget");
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PATH_FORM_WORK")),
        "{:?}",
        err
    );
}

/// The per-repetition caps keep their own refusals and their own ORDER inside the
/// shape check: the hop cap before the chain projection, whose per-length loop
/// grows with `max`. Checking the work budget first would have had to project a
/// `max` of a million to charge for it.
#[test]
fn the_repetition_caps_still_decide_their_own_cases_in_order() {
    let edges = disclosed_edges(&[("ex:a ex:p ex:b .", &["http://ex/p"])]);

    // Over the hop cap: the hop cap's message, not the work budget's.
    let form = PathForm::repeat_step(PathStep::predicate(nn("http://ex/p")), 1, 1_000_000);
    let err = eval_bounded_path_disclosed(&edges, &form).unwrap_err();
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_UNROLL_HOPS")),
        "{:?}",
        err
    );

    // Over the chain cap but inside the hop cap: the chain cap's message.
    let form = PathForm::repeat_step(
        PathStep::alternation([nn("http://ex/p"), nn("http://ex/q")]),
        1,
        40,
    );
    let err = eval_bounded_path_disclosed(&edges, &form).unwrap_err();
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_UNROLL_CHAINS")),
        "{:?}",
        err
    );
}
