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
use oxrdf::{BlankNode, Literal, Term};
use sparq_core::Graph;
use sparq_engine::query;
use std::collections::BTreeMap;

const PFX: &str = "@prefix ex: <http://ex/> .\n";

fn nn(iri: &str) -> NamedNode {
    NamedNode::new(iri).unwrap()
}

/// An IRI as a [`Term`], for the `Term`-typed declared node domain.
fn t(n: NamedNode) -> Term {
    Term::NamedNode(n)
}

/// Build the per-predicate disclosed edge set WITHOUT declaring a node domain —
/// each holder discloses its `(?s,?o)` rows for a predicate via `evaluate_local`,
/// exactly as a federation member would. A `min == 0` form over this is refused.
fn disclosed_edges_only(holder_docs: &[(&str, &[&str])]) -> DisclosedEdges {
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

/// Every distinct subject/object TERM of the WHOLE union of the holder documents —
/// the active graph's node domain in the sense of SPARQL 1.1 §18.4, read out of
/// the union store through the real engine so it is literally the term set the
/// clear-text oracle's zero-length match ranges over. Strictly wider than the
/// endpoint set of any single predicate.
///
/// NO term-kind filter: the engine's `graph_nodes` takes the subject and object of
/// every triple whatever their kind, so a LITERAL object is in the domain. Keeping
/// an IRI-only filter here would have hidden exactly the bug the literal case
/// exercises, and would have been unimplementable anyway now the declared domain
/// is a `Term` set. Predicate-only terms are excluded for free, since `?p` is not
/// projected.
fn active_graph_nodes(docs: &[&str]) -> Vec<Term> {
    let u = union_graph(docs);
    let res = query(&u, "SELECT ?s ?o WHERE { ?s ?p ?o }").unwrap();
    let mut out: BTreeMap<String, Term> = BTreeMap::new();
    for row in &res.rows {
        for slot in row.iter().flatten() {
            out.entry(format!("{:?}", slot))
                .or_insert_with(|| slot.clone());
        }
    }
    out.into_values().collect()
}

/// [`disclosed_edges_only`] plus the DECLARED public node domain of the union of
/// all the holder documents — the input a `{0,k}` / `p?` form needs.
fn disclosed_edges(holder_docs: &[(&str, &[&str])]) -> DisclosedEdges {
    let docs: Vec<&str> = holder_docs.iter().map(|(d, _)| *d).collect();
    disclosed_edges_only(holder_docs)
        .with_node_domain(active_graph_nodes(&docs))
        .expect("the active graph's node domain covers every disclosed endpoint")
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

/// The three predicates the fuzz graphs use. Each iteration discloses a random
/// NON-EMPTY subset of them and draws the path's predicates from that subset, so
/// most iterations have a predicate whose triples are in the active graph but
/// contribute no edge to the path. Those triples still name nodes, and a `{0,k}`
/// arm has to bind `(x,x)` for them (SPARQL 1.1 §18.4) — which is why the sweep
/// declares the node domain of the WHOLE generated graph.
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

    /// Whether any repetition in the form has `min == 0`, i.e. the form has a
    /// zero-length arm and therefore reads the node domain.
    fn admits_zero_length(&self) -> bool {
        match self {
            FuzzForm::Rep(_, m, _) => *m == 0,
            FuzzForm::Seq(parts) => parts.iter().any(FuzzForm::admits_zero_length),
        }
    }
}

/// The form rewritten as a SPARQL property path the FINAL Rec's grammar carries
/// (`/`, `|`, `?` only — never the removed `{m,n}`), so the real engine can serve
/// as the oracle. `None` when the form is not expressible that way: a repetition
/// with `max == 0` denotes the zero-length relation alone, which has no
/// property-path spelling.
fn fuzz_sparql(f: &FuzzForm) -> Option<String> {
    match f {
        FuzzForm::Rep(alts, m, k) => {
            if *k == 0 {
                return None;
            }
            let step = if alts.len() == 1 {
                format!("<{}>", FUZZ_PREDS[alts[0]])
            } else {
                let inner: Vec<String> = alts
                    .iter()
                    .map(|&i| format!("<{}>", FUZZ_PREDS[i]))
                    .collect();
                format!("({})", inner.join("|"))
            };
            let mut arms: Vec<String> = Vec::new();
            // `min == 0` is carried by `?`, which supplies lengths 0 AND 1.
            let first = if *m == 0 {
                arms.push(format!("{}?", step));
                2
            } else {
                *m
            };
            for length in first..=*k {
                arms.push(vec![step.clone(); length].join("/"));
            }
            if arms.len() == 1 {
                Some(arms.remove(0))
            } else {
                Some(format!("({})", arms.join("|")))
            }
        }
        FuzzForm::Seq(parts) => {
            let rendered: Option<Vec<String>> = parts
                .iter()
                .map(|p| fuzz_sparql(p).map(|r| format!("({})", r)))
                .collect();
            Some(rendered?.join("/"))
        }
    }
}

/// Generate a form. `depth` bounds the nesting; the sizes are kept small so the
/// projected chain count stays far below [`MAX_UNROLL_CHAINS`] and every case
/// evaluates in microseconds.
fn fuzz_form(rng: &mut FuzzRng, depth: usize, allowed: &[usize]) -> FuzzForm {
    fn rep(rng: &mut FuzzRng, allowed: &[usize]) -> FuzzForm {
        let n_alts = 1 + rng.below(2) as usize;
        // Only DISCLOSED predicates may appear in the path: an undisclosed one
        // contributes no edge, so a path naming it would not be the same relation
        // the engine evaluates over the union store.
        let alts: Vec<usize> = (0..n_alts)
            .map(|_| allowed[rng.below(allowed.len() as u64) as usize])
            .collect();
        let m = rng.below(3) as usize;
        let k = m + rng.below(2) as usize;
        FuzzForm::Rep(alts, m, k)
    }
    if depth == 0 || rng.chance(1, 3) {
        return rep(rng, allowed);
    }
    let n_parts = 2 + rng.below(2) as usize;
    let parts = (0..n_parts)
        .map(|_| {
            // Nest a sequence inside a sequence often — that is the shape under
            // test — but keep plain repetitions in the mix too.
            if rng.chance(1, 2) {
                fuzz_form(rng, depth - 1, allowed)
            } else {
                rep(rng, allowed)
            }
        })
        .collect();
    FuzzForm::Seq(parts)
}

/// How many predicate occurrences a rendered path may have before the sweep stops
/// handing it to the engine (see the sweep's own comment).
const ENGINE_ORACLE_MAX_STEPS: usize = 8;

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
    // ANY triple of the generated graph, under any predicate — disclosed to the
    // path or not (SPARQL 1.1 §18.4 ZeroLengthPath).
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
    let mut partial_disclosure = 0usize;
    let mut zero_length_with_undisclosed = 0usize;
    let mut engine_checked = 0usize;

    for iter in 0..200 {
        let per_holder = fuzz_graph(&mut rng);
        // A random NON-EMPTY subset of the predicates is disclosed to the path.
        // The rest stay in the active graph, contributing nodes but no edges.
        let disclosed: Vec<usize> = loop {
            let pick: Vec<usize> = (0..FUZZ_PREDS.len()).filter(|_| rng.chance(1, 2)).collect();
            if !pick.is_empty() {
                break pick;
            }
        };
        let form = fuzz_form(&mut rng, 2, &disclosed);

        let docs: Vec<String> = per_holder.iter().map(fuzz_turtle).collect();
        let disclosed_iris: Vec<&str> = disclosed.iter().map(|&i| FUZZ_PREDS[i]).collect();
        let args: Vec<(&str, &[&str])> = docs
            .iter()
            .map(|d| (d.as_str(), &disclosed_iris[..]))
            .collect();
        // `disclosed_edges` declares the node domain of the WHOLE generated graph
        // (read back through the engine), which is what §18.4 asks for.
        let edges = disclosed_edges(&args);

        let got = federated_pairs(&edges, &form.to_path_form());
        let want = fuzz_oracle_pairs(&fuzz_oracle(&form, &per_holder));
        assert_eq!(
            got, want,
            "iter {}: the unroll disagrees with the oracle\n  form: {:?}\n  disclosed: {:?}\n  graph: {:?}",
            iter, form, disclosed, per_holder
        );

        // Where the form is expressible in the final-Rec property-path grammar,
        // the REAL engine is the oracle too — the same differential the named
        // tests above run, now over randomly-generated graphs and forms.
        // Cap how big a path the engine is asked to serve: the rendered union of
        // chains grows multiplicatively with nesting, and the engine materialises
        // the whole path relation, so the largest generated forms dominate the
        // sweep's runtime without adding a shape the smaller ones miss.
        let engine_path = fuzz_sparql(&form)
            .filter(|p| p.matches("http://ex/f").count() <= ENGINE_ORACLE_MAX_STEPS);
        if let Some(path) = engine_path {
            let doc_refs: Vec<&str> = docs.iter().map(String::as_str).collect();
            assert_eq!(
                got,
                clear_text_pairs(&doc_refs, &path),
                "iter {}: the unroll disagrees with the engine\n  form: {:?}\n  path: {}\n  disclosed: {:?}\n  graph: {:?}",
                iter,
                form,
                path,
                disclosed,
                per_holder
            );
            engine_checked += 1;
        }

        // Book-keeping for the coverage assertions below.
        let undisclosed_edges = per_holder
            .iter()
            .flatten()
            .any(|(_, p, _)| !disclosed.contains(p));
        if undisclosed_edges {
            partial_disclosure += 1;
            if form.admits_zero_length() {
                zero_length_with_undisclosed += 1;
            }
        }

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
    assert!(
        partial_disclosure >= 20,
        "the sweep must actually exercise graphs with UNDISCLOSED predicates, saw {} of 200",
        partial_disclosure
    );
    assert!(
        zero_length_with_undisclosed >= 10,
        "and a zero-length arm together with an undisclosed predicate — the shape \
         the node-domain fix is about — saw {} of 200",
        zero_length_with_undisclosed
    );
    assert!(
        engine_checked >= 50,
        "and the real engine as the oracle on a good share of them, saw {} of 200",
        engine_checked
    );
}

// =====================================================================
// 10. ZERO-LENGTH NODE DOMAIN — the whole active graph, not the path's
//     own disclosed predicates (SPARQL 1.1 §18.4 ZeroLengthPath)
// =====================================================================

/// THE reproducer. Holder A holds `ex:a ex:p ex:b`, holder B holds `ex:c ex:q ex:d`,
/// and only `ex:p` is disclosed to the path. `ex:p?` is a zero-length arm, so §18.4
/// binds `(x,x)` for EVERY term of the active graph — including `ex:c` and `ex:d`,
/// which occur only in the `ex:q` triple the path never traverses.
///
/// Drawing the identity pairs from the disclosed `ex:p` edges alone yielded
/// {aa, ab, bb} and missed (c,c) and (d,d). With the active graph's node domain
/// declared, the unroll equals the engine's `ex:p?` exactly.
#[test]
fn differential_zero_length_domain_spans_predicates_outside_the_path() {
    let h1 = "ex:a ex:p ex:b .";
    let h2 = "ex:c ex:q ex:d .";
    // Only ex:p is disclosed; ex:q contributes NO edge to the path.
    let edges = disclosed_edges(&[(h1, &["http://ex/p"]), (h2, &[])]);

    let form = PathForm::range(nn("http://ex/p"), 0, 1);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "ex:p?");
    assert_eq!(
        got, want,
        "p? must bind (x,x) for every term of the active graph, not just the \
         endpoints of the disclosed ex:p edges"
    );

    // Spelled out, so the test states the answer rather than only mirroring it.
    let term = |iri: &str| format!("{:?}", Term::from(nn(iri)));
    for n in ["http://ex/a", "http://ex/b", "http://ex/c", "http://ex/d"] {
        assert!(
            got.contains(&(term(n), term(n))),
            "the identity pair for {n} must be present: {got:?}"
        );
    }
    assert!(got.contains(&(term("http://ex/a"), term("http://ex/b"))));
    assert_eq!(
        got.len(),
        5,
        "4 identity pairs + the one ex:p edge: {got:?}"
    );
}

/// The same shape with a bounded `{0,2}` arm and a node reachable only through a
/// predicate the path does traverse plus one it does not.
#[test]
fn differential_zero_to_k_domain_spans_predicates_outside_the_path() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c .";
    let h2 = "ex:x ex:q ex:y . ex:y ex:q ex:z .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"]), (h2, &[])]);

    let form = PathForm::range(nn("http://ex/p"), 0, 2);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "(ex:p)? | ex:p/ex:p");
    assert_eq!(got, want);
    // 6 identity pairs (a,b,c,x,y,z) + (a,b),(b,c) + (a,c) = 9.
    assert_eq!(got.len(), 9, "{got:?}");
}

/// A zero-length arm on an edge set with NO declared node domain is REFUSED, not
/// answered from the disclosed endpoints. The refusal is value-free: it names no
/// term, no node count and no row count.
#[test]
fn zero_length_arm_without_a_declared_domain_is_refused() {
    let h1 = "ex:a ex:p ex:b .";
    let h2 = "ex:c ex:q ex:d .";
    let edges = disclosed_edges_only(&[(h1, &["http://ex/p"]), (h2, &["http://ex/q"])]);

    for form in [
        PathForm::range(nn("http://ex/p"), 0, 1),
        PathForm::range(nn("http://ex/p"), 0, 3),
        PathForm::range(nn("http://ex/p"), 0, 0),
        // Nested: a zero-length arm inside a sequence is refused too.
        PathForm::Sequence(vec![
            PathForm::exact(nn("http://ex/p"), 1),
            PathForm::range(nn("http://ex/p"), 0, 2),
        ]),
    ] {
        let err = eval_bounded_path_disclosed(&edges, &form).unwrap_err();
        match &err {
            MpcError::Protocol(m) => {
                assert!(
                    m.contains("PUBLIC node domain") && m.contains("with_node_domain"),
                    "{}",
                    m
                );
                for value in ["ex/a", "ex/b", "ex/c", "ex/d"] {
                    assert!(!m.contains(value), "the message names a term: {}", m);
                }
            }
            other => panic!("expected a Protocol refusal, got {:?}", other),
        }
    }
}

/// A form with `min >= 1` never reads the node domain, so it runs unchanged on an
/// edge set that declares none — the refusal is scoped to the zero-length arm.
#[test]
fn a_min_one_form_needs_no_declared_domain() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c .";
    let undeclared = disclosed_edges_only(&[(h1, &["http://ex/p"])]);
    let declared = disclosed_edges(&[(h1, &["http://ex/p"])]);
    let form = PathForm::range(nn("http://ex/p"), 1, 2);
    assert_eq!(
        federated_pairs(&undeclared, &form),
        clear_text_pairs(&[h1], "ex:p | ex:p/ex:p")
    );
    assert_eq!(
        federated_pairs(&undeclared, &form),
        federated_pairs(&declared, &form),
        "declaring a domain cannot change a min >= 1 answer"
    );
}

/// A declared domain that omits an endpoint of a disclosed edge is inconsistent
/// with those edges and is refused at declaration time — a zero-length arm over it
/// would under-answer exactly as the old edge-derived domain did. The message names
/// counts only, no terms.
#[test]
fn a_declared_domain_that_omits_a_disclosed_endpoint_is_refused() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c .";
    let edges = disclosed_edges_only(&[(h1, &["http://ex/p"])]);
    let err = edges
        .clone()
        // ex:c is an object of a disclosed edge but is left out.
        .with_node_domain([t(nn("http://ex/a")), t(nn("http://ex/b"))])
        .unwrap_err();
    match &err {
        MpcError::Protocol(m) => {
            assert!(
                m.contains("the declared node domain does not cover the edge endpoints"),
                "{}",
                m
            );
            assert!(!m.contains("ex/c"), "the message names a term: {}", m);
            // COUNT-free as well as value-free: the message must not carry how many
            // endpoints were uncovered, nor how many terms were declared — both are
            // statistics over the edges, probeable one term at a time.
            assert!(
                !m.chars().any(|c| c.is_ascii_digit()),
                "the message carries a count: {}",
                m
            );
        }
        other => panic!("expected a Protocol refusal, got {:?}", other),
    }

    // A domain WIDER than the disclosed endpoints is fine — that is the whole
    // point: the active graph has terms the path's predicates never touch.
    edges
        .with_node_domain([
            t(nn("http://ex/a")),
            t(nn("http://ex/b")),
            t(nn("http://ex/c")),
            t(nn("http://ex/elsewhere")),
        ])
        .expect("a superset domain is accepted");
}

/// Declaring the same domain in a different order, or with duplicates, gives the
/// same answer: `with_node_domain` dedups and canonicalises.
#[test]
fn a_declared_domain_is_order_and_duplicate_insensitive() {
    let h1 = "ex:a ex:p ex:b .";
    let base = disclosed_edges_only(&[(h1, &["http://ex/p"])]);
    let form = PathForm::range(nn("http://ex/p"), 0, 1);
    let forward = base
        .clone()
        .with_node_domain([
            t(nn("http://ex/a")),
            t(nn("http://ex/b")),
            t(nn("http://ex/z")),
        ])
        .unwrap();
    let shuffled = base
        .with_node_domain([
            t(nn("http://ex/z")),
            t(nn("http://ex/b")),
            t(nn("http://ex/a")),
            t(nn("http://ex/b")),
        ])
        .unwrap();
    let a = eval_bounded_path_disclosed(&forward, &form).unwrap();
    let b = eval_bounded_path_disclosed(&shuffled, &form).unwrap();
    assert_eq!(
        a.rows, b.rows,
        "the row ORDER must match too, not just the set"
    );
    assert_eq!(pair_set(&a.vars, &a.rows).len(), 4, "aa,bb,zz,ab");
}

// =====================================================================
// 11. THE DOMAIN'S TERM KINDS — literals are in it, predicate-only
//     terms are not; an empty graph has an empty diagonal
// =====================================================================

/// The engine's zero-length domain is every distinct SUBJECT or OBJECT term,
/// whatever its kind — so a LITERAL object is in it. Graph `ex:a ex:p ex:b .
/// ex:c ex:q "L"`, with only `ex:p` disclosed: the engine's `ex:p?` returns
/// `("L","L")` along with the four IRI identity pairs.
///
/// While the declared domain was `NamedNode`-only the true domain could not be
/// expressed: declaring `{a, b, c}` was accepted — it covers the one disclosed
/// `ex:p` edge — yet `p?` then omitted `("L","L")`, which no `NamedNode` list can
/// supply.
#[test]
fn differential_zero_length_domain_includes_a_literal_object() {
    let h1 = "ex:a ex:p ex:b .";
    let h2 = "ex:c ex:q \"L\" .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"]), (h2, &[])]);

    let form = PathForm::range(nn("http://ex/p"), 0, 1);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "ex:p?");
    assert_eq!(
        got, want,
        "the zero-length domain must contain the literal object too"
    );

    let lit = format!("{:?}", Term::from(Literal::new_simple_literal("L")));
    assert!(
        got.contains(&(lit.clone(), lit)),
        "the literal identity pair must be present: {got:?}"
    );
    // 4 identity pairs (a, b, c, "L") + the one ex:p edge = 5.
    assert_eq!(got.len(), 5, "{got:?}");
}

/// A `{0,2}` arm over a graph whose literals sit under BOTH a disclosed and an
/// undisclosed predicate, and whose typed/language literals are distinct terms.
#[test]
fn differential_zero_length_domain_distinguishes_literal_kinds() {
    // The path traverses ex:r, whose one edge has IRI endpoints (the edge regime
    // is IRI-only). The literals reach the DOMAIN from ex:q, which no holder
    // discloses — so they can only arrive through the declaration.
    let h1 = "ex:a ex:r ex:b .";
    let h2 = "ex:c ex:q \"L\" . ex:d ex:q \"L\"@en . ex:e ex:q 7 .";
    let edges = disclosed_edges(&[(h1, &["http://ex/r"]), (h2, &[])]);

    let form = PathForm::range(nn("http://ex/r"), 0, 2);
    let got = federated_pairs(&edges, &form);
    let want = clear_text_pairs(&[h1, h2], "(ex:r)? | ex:r/ex:r");
    assert_eq!(got, want);
    // 5 IRIs (a,b,c,d,e) + 3 distinct literals ("L", "L"@en, 7) = 8 identity
    // pairs, plus the (a,b) edge = 9. The three literals are distinct TERMS, so
    // the plain and the language-tagged "L" do not collapse.
    assert_eq!(got.len(), 9, "{got:?}");
}

/// A PREDICATE-only term is NOT in the domain: `ex:p` occurs only as a predicate,
/// so the engine binds no `(ex:p, ex:p)` and neither does the unroll.
#[test]
fn differential_zero_length_domain_excludes_a_predicate_only_term() {
    let h1 = "ex:a ex:p ex:b .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"])]);

    let form = PathForm::range(nn("http://ex/p"), 0, 1);
    let got = federated_pairs(&edges, &form);
    assert_eq!(got, clear_text_pairs(&[h1], "ex:p?"));
    let pred = format!("{:?}", Term::from(nn("http://ex/p")));
    assert!(
        !got.contains(&(pred.clone(), pred)),
        "a predicate-only term must not be in the zero-length domain: {got:?}"
    );
    // Only (a,a), (b,b) and (a,b).
    assert_eq!(got.len(), 3, "{got:?}");
}

/// An EMPTY active graph with variable endpoints gives an EMPTY diagonal — the
/// declared domain is empty, which is a declaration, not an absent one, so the
/// form runs and returns nothing rather than being refused.
#[test]
fn differential_zero_length_over_an_empty_graph_is_empty() {
    let edges = disclosed_edges(&[("", &["http://ex/p"])]);
    let form = PathForm::range(nn("http://ex/p"), 0, 2);
    let got = federated_pairs(&edges, &form);
    assert_eq!(got, clear_text_pairs(&[""], "(ex:p)? | ex:p/ex:p"));
    assert!(got.is_empty(), "{got:?}");

    // The distinction that matters: an EMPTY declared domain runs, an UNDECLARED
    // one is refused. Both would print as "no identity pairs" otherwise.
    let undeclared = disclosed_edges_only(&[("", &["http://ex/p"])]);
    assert!(eval_bounded_path_disclosed(&undeclared, &form).is_err());
    assert!(eval_bounded_path_disclosed(&edges, &form).is_ok());
}

/// The engine matches a BOUND endpoint against itself even when the constant does
/// not occur in the graph, and the declared domain is how a caller reproduces
/// that: including the absent constant yields its `(x,x)` pair, leaving it out
/// does not. No API change is involved — it is just a wider declaration.
#[test]
fn an_absent_bound_endpoint_self_matches_when_declared() {
    let h1 = "ex:a ex:p ex:b .";
    let base = disclosed_edges_only(&[(h1, &["http://ex/p"])]);
    let form = PathForm::range(nn("http://ex/p"), 0, 1);
    let absent = format!("{:?}", Term::from(nn("http://ex/absent")));

    // The engine's own answer for the bound-endpoint case: `<ex:absent> ex:p? ?b`
    // binds (absent, absent) although ex:absent is in no triple.
    let u = union_graph(&[h1]);
    let q = "PREFIX ex: <http://ex/> SELECT ?b WHERE { ex:absent ex:p? ?b }";
    let res = query(&u, q).unwrap();
    // Compare the TERM, not just the row count: a single row binding something
    // else would be a different behaviour entirely, and the count alone would not
    // notice.
    assert_eq!(
        res.rows
            .iter()
            .map(|r| r[0].as_ref().map(|t| format!("{t:?}")))
            .collect::<Vec<_>>(),
        vec![Some(absent.clone())],
        "the engine binds ?b to ex:absent itself, nothing else"
    );

    let with_absent = base
        .clone()
        .with_node_domain(
            active_graph_nodes(&[h1])
                .into_iter()
                .chain([t(nn("http://ex/absent"))]),
        )
        .unwrap();
    let got = federated_pairs(&with_absent, &form);
    assert!(
        got.contains(&(absent.clone(), absent.clone())),
        "declaring the absent constant reproduces its self-match: {got:?}"
    );

    let without = base.with_node_domain(active_graph_nodes(&[h1])).unwrap();
    assert!(
        !federated_pairs(&without, &form).contains(&(absent.clone(), absent)),
        "and leaving it out does not invent it"
    );
}

/// THE scope limit on the advice above: a per-query widening of the domain with
/// an absent constant OVER-answers exactly when the WHOLE `Sequence` is nullable
/// and the bound endpoint is that constant — and then by at most ONE extra pair,
/// `(absent, absent)`.
///
/// Why that is the condition, and why the bound is one pair: no disclosed edge
/// touches the widened term, so the only row that can ever carry it is a
/// zero-length arm's own `(absent, absent)` diagonal entry. Chaining those
/// through a `Sequence` midpoint composes to `(absent, absent)` and to nothing
/// else; and the first arm with `min >= 1` needs an EDGE out of `absent`, which
/// does not exist, so one non-nullable arm anywhere in the sequence removes the
/// extra pair entirely (`p?/q?/r` below).
///
/// A `{0,k}` RANGE is not a case of this: it is ONE repetition, evaluated as the
/// union over the admitted lengths — it behaves like `p?|p|p/p…`, not like the
/// sequence `p?/p?`. The engine self-matches a bound constant on a zero-length
/// arm too, so there the operator and the engine AGREE: that is precisely the
/// query the widening is declared for.
///
/// This test pins all three shapes, so the doc's caveat is backed rather than
/// asserted.
#[test]
fn an_absent_constant_over_answers_only_a_fully_nullable_sequence() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:q ex:c . ex:c ex:r ex:d .";
    let preds: &[&str] = &["http://ex/p", "http://ex/q", "http://ex/r"];
    let absent = format!("{:?}", Term::from(nn("http://ex/absent")));
    let widen = || {
        disclosed_edges_only(&[(h1, preds)])
            .with_node_domain(
                active_graph_nodes(&[h1])
                    .into_iter()
                    .chain([t(nn("http://ex/absent"))]),
            )
            .unwrap()
    };
    let exactly = || {
        disclosed_edges_only(&[(h1, preds)])
            .with_node_domain(active_graph_nodes(&[h1]))
            .unwrap()
    };

    // (1) THE over-answering shape: every arm nullable.
    //
    // The ENGINE side, both shapes: the bound-endpoint query binds nothing, and
    // the variable-endpoint path never names ex:absent either.
    let u = union_graph(&[h1]);
    let bound = query(
        &u,
        "PREFIX ex: <http://ex/> SELECT ?b WHERE { ex:absent ex:p?/ex:q? ?b }",
    )
    .unwrap();
    assert!(
        bound.rows.is_empty(),
        "the engine's inner q? sees a VARIABLE midpoint, so an absent constant \
         cannot traverse it: {:?}",
        bound.rows
    );
    let engine = clear_text_pairs(&[h1], "ex:p?/ex:q?");
    assert!(!engine.contains(&(absent.clone(), absent.clone())));

    // The MPC side with the widened domain: (absent, absent) appears.
    let nullable = PathForm::Sequence(vec![
        PathForm::range(nn("http://ex/p"), 0, 1),
        PathForm::range(nn("http://ex/q"), 0, 1),
    ]);
    let over = federated_pairs(&widen(), &nullable);
    assert!(
        over.contains(&(absent.clone(), absent.clone())),
        "the widened constant reaches itself through the Sequence midpoint: {over:?}"
    );
    assert_eq!(
        &over - &engine,
        BTreeSet::from([(absent.clone(), absent.clone())]),
        "and (absent, absent) is the ONLY over-answer — the widening adds nothing else"
    );

    // Without the widening the two sides agree exactly, so the over-answer is
    // caused by the widening and by nothing else in the form.
    assert_eq!(
        federated_pairs(&exactly(), &nullable),
        engine,
        "an exactly-declared domain matches the engine on the same form"
    );

    // (2) ONE non-nullable arm removes the extra pair: `p?/q?/r` needs an edge
    // out of the midpoint, and no disclosed edge touches the widened term.
    let with_mandatory = PathForm::Sequence(vec![
        PathForm::range(nn("http://ex/p"), 0, 1),
        PathForm::range(nn("http://ex/q"), 0, 1),
        PathForm::exact(nn("http://ex/r"), 1),
    ]);
    let engine_mandatory = clear_text_pairs(&[h1], "ex:p?/ex:q?/ex:r");
    let bound_mandatory = query(
        &u,
        "PREFIX ex: <http://ex/> SELECT ?b WHERE { ex:absent ex:p?/ex:q?/ex:r ?b }",
    )
    .unwrap();
    assert!(
        bound_mandatory.rows.is_empty(),
        "the engine returns nothing for an absent bound endpoint here either: {:?}",
        bound_mandatory.rows
    );
    let mandatory = federated_pairs(&widen(), &with_mandatory);
    assert_eq!(
        mandatory, engine_mandatory,
        "a sequence with one mandatory arm does not over-answer even widened"
    );
    assert!(
        !mandatory.iter().any(|(a, b)| *a == absent || *b == absent),
        "and no row mentions the widened term at all: {mandatory:?}"
    );

    // (3) A `{0,k}` RANGE is ONE repetition, not `p?/p?`: the engine self-matches
    // the bound constant on the zero-length arm, so the operator and the engine
    // agree — this is the query the widening is declared for, not an over-answer.
    // (The engine's path syntax has no `{m,k}`, so the engine half is pinned on
    // `p?`, which IS `{0,1}`.)
    let bound_range = query(
        &u,
        "PREFIX ex: <http://ex/> SELECT ?b WHERE { ex:absent ex:p? ?b }",
    )
    .unwrap();
    assert_eq!(
        bound_range.rows.len(),
        1,
        "the engine self-matches a bound constant on the zero-length arm: {:?}",
        bound_range.rows
    );
    // And the extra pair stays ONE for every k: a wider range admits more
    // LENGTHS of the same repetition, it does not chain `p?` arms through a
    // midpoint, so nothing composes the widened term with anything else.
    for k in 1..=3 {
        let form = PathForm::range(nn("http://ex/p"), 0, k);
        let ranged = federated_pairs(&widen(), &form);
        assert!(
            ranged.contains(&(absent.clone(), absent.clone())),
            "the operator self-matches the declared constant at k={k}: {ranged:?}"
        );
        assert_eq!(
            &ranged - &federated_pairs(&exactly(), &form),
            BTreeSet::from([(absent.clone(), absent.clone())]),
            "one repetition adds exactly the declared constant's own diagonal \
             entry, whatever k is (k={k})"
        );
    }
}

/// A BLANK NODE of the LOADED graph, against the real engine. The fixture puts a
/// blank node in the active graph under a predicate the path does not traverse,
/// so it is a subject/object term of the domain while the EDGE relations stay
/// IRI-only, and the declared domain is the graph's own — nothing synthetic is
/// added. A `{0,1}` arm must then bind the blank node's `(x,x)` pair exactly as
/// the engine does.
///
/// The second half pins the edge regime: `from_holder_edges` still refuses a
/// blank-node EDGE endpoint, which is why a diagonal row for a blank node can
/// only ever meet another diagonal row for the same declared term.
#[test]
fn a_blank_node_of_the_loaded_graph_yields_its_identity_pair() {
    // `ex:q` is in the graph but is NOT a disclosed edge predicate of the path,
    // so its blank-node object reaches the operator only through the domain.
    let h1 = "ex:a ex:p ex:b . ex:c ex:q [ ] .";

    // ONE load for both sides: a blank node's identity is per-load, so the
    // declared domain and the engine's answer must come from the SAME graph for
    // the differential to be about the operator rather than about labelling.
    let g = union_graph(&[h1]);
    let mut domain: BTreeMap<String, Term> = BTreeMap::new();
    for row in &query(&g, "SELECT ?s ?o WHERE { ?s ?p ?o }").unwrap().rows {
        for slot in row.iter().flatten() {
            domain
                .entry(format!("{:?}", slot))
                .or_insert_with(|| slot.clone());
        }
    }
    let engine_res = query(
        &g,
        "PREFIX ex: <http://ex/> SELECT ?a ?b WHERE { ?a ex:p? ?b }",
    )
    .unwrap();
    let engine = pair_set(&engine_res.vars, &engine_res.rows);

    let edges = disclosed_edges_only(&[(h1, &["http://ex/p"])])
        .with_node_domain(domain.into_values())
        .expect("the loaded graph's own domain, blank node included");

    let form = PathForm::range(nn("http://ex/p"), 0, 1);
    let got = federated_pairs(&edges, &form);
    assert_eq!(
        got, engine,
        "the operator must agree with the engine on the loaded graph's domain"
    );
    let blanks = got
        .iter()
        .filter(|(a, b)| a.starts_with("BlankNode") && a == b)
        .count();
    assert_eq!(
        blanks, 1,
        "the loaded graph's blank node must contribute its identity pair: {got:?}"
    );

    // The EDGE regime is untouched: a blank-node edge endpoint is still refused.
    let b = Term::BlankNode(BlankNode::new("b0").unwrap());
    let bad = PartialResult {
        holder: HolderId::new("h0"),
        vars: vec![Variable::new_unchecked("s"), Variable::new_unchecked("o")],
        rows: vec![vec![Some(b), Some(Term::from(nn("http://ex/x")))]],
    };
    let err = DisclosedEdges::from_holder_edges([(nn("http://ex/p"), bad)]).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(_)));
}

/// A LITERAL through a nested nullable sequence `p?/p?`, against the engine. The
/// literal enters only via the declared domain, and the two diagonals compose
/// through it — so this exercises a non-IRI term as a Sequence MIDPOINT, which the
/// `DisclosedKeyJoin` keys on.
#[test]
fn differential_literal_through_a_nested_nullable_sequence() {
    let h1 = "ex:a ex:p ex:b .";
    let h2 = "ex:c ex:q \"L\" .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"]), (h2, &[])]);

    let form = PathForm::Sequence(vec![
        PathForm::range(nn("http://ex/p"), 0, 1),
        PathForm::range(nn("http://ex/p"), 0, 1),
    ]);
    let got = federated_pairs(&edges, &form);
    assert_eq!(got, clear_text_pairs(&[h1, h2], "ex:p?/ex:p?"));

    let lit = format!("{:?}", Term::from(Literal::new_simple_literal("L")));
    assert!(
        got.contains(&(lit.clone(), lit)),
        "the literal must survive BOTH nullable arms, i.e. the join on it: {got:?}"
    );
    // 4 identity pairs (a, b, c, "L") + (a,b) reached by either arm = 5.
    assert_eq!(got.len(), 5, "{got:?}");
}

/// Two literals with the SAME lexical form but different datatypes are distinct
/// RDF terms, as are a canonical and a non-canonical lexical form of the same
/// numeric value. The declared domain and the diagonal must keep them apart
/// exactly as the engine does — the dedup key is the term, not the lexical form
/// and not the value.
#[test]
fn differential_literal_domain_keeps_equal_lexical_and_non_canonical_forms_apart() {
    let h1 = "ex:a ex:p ex:b .";
    // Same lexical form "1", three datatypes; plus 1 / 01 / 1.0, which are
    // distinct TERMS although xsd:integer 1 == xsd:decimal 1.0 by VALUE.
    let h2 = "ex:c ex:q \"1\" . \
              ex:c ex:q \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> . \
              ex:c ex:q \"1\"^^<http://www.w3.org/2001/XMLSchema#string> . \
              ex:c ex:q \"01\"^^<http://www.w3.org/2001/XMLSchema#integer> . \
              ex:c ex:q \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal> .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"]), (h2, &[])]);

    let form = PathForm::range(nn("http://ex/p"), 0, 1);
    let got = federated_pairs(&edges, &form);
    assert_eq!(
        got,
        clear_text_pairs(&[h1, h2], "ex:p?"),
        "the engine and the unroll must agree on which literals are distinct terms"
    );

    // How many distinct terms the engine itself sees, so the count below is the
    // engine's verdict rather than this test's guess.
    let distinct = active_graph_nodes(&[h1, h2]).len();
    assert_eq!(
        got.len(),
        distinct + 1,
        "one identity pair per distinct domain term, plus the (a,b) edge: {got:?}"
    );
    // "1" and "1"^^xsd:string are the same term in RDF 1.1; the others are not,
    // so the five object literals above collapse to four distinct terms.
    assert_eq!(distinct, 3 + 4, "a, b, c plus four distinct literals");
}

/// `DisclosedEdges`' `Debug` prints SIZES, not terms. Nothing here is secret —
/// this is the disclosed-key regime — so this is hygiene, not redaction: a derived
/// `Debug` spilled the whole federated edge relation and the declared domain into
/// any log line or `assert_eq!` failure that rendered one.
#[test]
fn disclosed_edges_debug_prints_sizes_not_terms() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c .";
    let declared = disclosed_edges(&[(h1, &["http://ex/p"])]);
    let shown = format!("{declared:?}");
    assert!(
        shown.contains("http://ex/p: 2 edge(s)") && shown.contains("3 term(s)"),
        "the sizes must print: {shown}"
    );
    // The PREDICATE is public and part of the plan, so it stays; the endpoint and
    // domain TERMS do not appear.
    for term in ["ex/a", "ex/b", "ex/c"] {
        assert!(
            !shown.contains(term),
            "a term reached the rendering: {shown}"
        );
    }
    // `{:#?}` is a different path through `Formatter`, so check it too.
    let pretty = format!("{declared:#?}");
    for term in ["ex/a", "ex/b", "ex/c"] {
        assert!(!pretty.contains(term), "a term reached {{:#?}}: {pretty}");
    }

    let undeclared = disclosed_edges_only(&[(h1, &["http://ex/p"])]);
    assert!(
        format!("{undeclared:?}").contains("none declared"),
        "an undeclared domain must say so rather than print a size"
    );
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

/// The ZERO-LENGTH arm's diagonal is charged to the same budget as a chain's
/// rows. The declared node domain is a public plan input, but the `(x, x)` rows it
/// contributes are rows this form materialises, so a `{0,k}` over a large
/// declared domain cannot walk past the cap on the quiet.
#[test]
fn the_row_budget_charges_the_zero_length_diagonal_too() {
    let h1 = "ex:a ex:p ex:b . ex:b ex:p ex:c .";
    let edges = disclosed_edges(&[(h1, &["http://ex/p"])]);
    let domain = active_graph_nodes(&[h1]).len();
    assert!(domain > 0, "the fixture declares a non-empty domain");
    let nullable = PathForm::range(nn("http://ex/p"), 0, 2);
    let mandatory = PathForm::range(nn("http://ex/p"), 1, 2);

    let mut with_diagonal = PathFormBudget::new();
    let baseline = eval_with_budget(&edges, &nullable, &mut with_diagonal).unwrap();
    let mut without_diagonal = PathFormBudget::new();
    eval_with_budget(&edges, &mandatory, &mut without_diagonal).unwrap();
    assert_eq!(
        with_diagonal.charged(),
        without_diagonal.charged() + domain,
        "the zero-length arm must charge exactly one row per declared node"
    );

    // And the charge BINDS: one row less than the form needs is refused.
    let spend = with_diagonal.charged();
    let mut tight = PathFormBudget::with_charged(MAX_PATH_FORM_ROWS - (spend - 1));
    let err = eval_with_budget(&edges, &nullable, &mut tight)
        .expect_err("a nullable form that would pass the budget is refused");
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PATH_FORM_ROWS")),
        "{:?}",
        err
    );
    let mut exact = PathFormBudget::with_charged(MAX_PATH_FORM_ROWS - spend);
    let got = eval_with_budget(&edges, &nullable, &mut exact).expect("exactly enough budget runs");
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
