use super::*;
use sparq_core::Graph;

fn var(s: &str) -> Variable {
    Variable::new_unchecked(s)
}

/// `term_id` for a plain literal string via a scratch graph so we get a real
/// dictionary ID. The same graph must be used for every ID we want to join.
fn scratch() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://ex/> .\
             ex:a ex:p \"v1\" .\
             ex:b ex:p \"v2\" .\
             ex:c ex:p \"v3\" .",
        "turtle",
    )
    .unwrap()
}

/// Resolve an IRI to its dictionary `Id` in the scratch graph. PANICS if the term
/// is absent (a test-setup mistake): a silent `NO_ID` fallback would masquerade as
/// an "unbound" sentinel and mask a typo, producing false pass/fail in the MINUS
/// compatibility tests. The IRI MUST be one interned by `scratch()`.
fn id(g: &Graph, iri: &str) -> Id {
    use oxrdf::{NamedNode, Term};
    g.id_of(&Term::NamedNode(NamedNode::new(iri).unwrap()))
        .unwrap_or_else(|| panic!("test-setup error: IRI {} is not present in the scratch graph", iri))
}

/// Disjoint variable domains: left has {?s} only, right has {?x} only (no overlap).
/// MINUS must return the left unchanged (fast path: `shared.is_empty()` → return left).
#[test]
fn minus_disjoint_domains_returns_left_unchanged() {
    let g = scratch();
    let (a, b, c) = (id(&g, "http://ex/a"), id(&g, "http://ex/b"), id(&g, "http://ex/c"));
    let left = Bindings::unsorted(vec![var("s")], vec![Row::from_slice(&[a]), Row::from_slice(&[b])]);
    let right = Bindings::unsorted(vec![var("x")], vec![Row::from_slice(&[c])]);
    let result = minus_bindings(left, right);
    // No shared variable → nothing removed.
    assert_eq!(result.rows.len(), 2, "disjoint domains: MINUS is a no-op, expected 2 rows");
}

/// Fully-bound shared variable (fast path: hash-table lookup).
/// Left has rows {s=a, s=b}; right has {s=a}. Fast path removes the a row.
#[test]
fn minus_fast_path_removes_compatible_row() {
    let g = scratch();
    let (a, b) = (id(&g, "http://ex/a"), id(&g, "http://ex/b"));
    let left = Bindings::unsorted(vec![var("s")], vec![Row::from_slice(&[a]), Row::from_slice(&[b])]);
    let right = Bindings::unsorted(vec![var("s")], vec![Row::from_slice(&[a])]);
    let result = minus_bindings(left, right);
    // a is compatible → removed; b is not in right → kept.
    assert_eq!(result.rows.len(), 1, "fast path must remove compatible row a");
    // The remaining row should be b.
    assert_eq!(result.rows[0][0], b, "remaining row should have s=b");
}

/// General path: the left row has an UNBOUND shared variable (`?t = NO_ID`) alongside
/// a BOUND shared variable (`?s`) whose value agrees with the right row. The unbound
/// `?t` forces `minus_bindings` off the fast path into the per-row compatibility scan.
///
/// SPARQL MINUS (§18.5): a left row is removed iff some right row is compatible AND
/// their bound domains overlap on ≥1 shared variable. Here the domains overlap on `?s`
/// (both bound, `a == a`); `?t` is unbound on the left so it is skipped (compatibility
/// only constrains variables bound on BOTH sides). Overlap holds ⇒ the row IS removed.
#[test]
fn minus_general_path_bound_overlap_removes_despite_unbound_var() {
    let g = scratch();
    let a = id(&g, "http://ex/a");
    // Left: {?s = a, ?t = NO_ID (unbound)}; Right: {?s = a, ?t = a (bound)}.
    let left = Bindings::unsorted(vec![var("s"), var("t")], vec![Row::from_slice(&[a, NO_ID])]);
    let right = Bindings::unsorted(vec![var("s"), var("t")], vec![Row::from_slice(&[a, a])]);
    let result = minus_bindings(left, right);
    // Overlap on ?s (both bound, equal) ⇒ compatible ⇒ removed; the unbound ?t is
    // skipped by the "both sides bound" guard and neither blocks nor causes removal.
    assert_eq!(result.rows.len(), 0, "bound overlap on ?s removes the row even though ?t is unbound on the left");
}

/// General path (unbound `?t` on the left forces the per-row compatibility scan off
/// the fast path): the shared variable `?s` is BOUND on both sides but to DIFFERENT
/// ids (`a` on the left, `b` on the right); `?t` is unbound on both sides.
///
/// SPARQL MINUS (§18.5): remove the left row iff some right row is compatible AND the
/// bound domains overlap on ≥1 shared variable. The domains DO overlap on `?s` (bound
/// on both sides), but the values disagree (`a != b`), so the rows are INCOMPATIBLE —
/// nothing is removed and the left row is KEPT. (`?t`, unbound on both sides, is in
/// neither solution's domain and cannot force removal.)
#[test]
fn minus_general_path_incompatible_bound_shared_var_keeps_row() {
    let g = scratch();
    let a = id(&g, "http://ex/a");
    let b = id(&g, "http://ex/b");
    // Left: {?s=a, ?t=NO_ID}; Right: {?s=b, ?t=NO_ID}.
    // ?s=a vs ?s=b: BOTH bound but NOT equal → incompatible → row KEPT.
    let left = Bindings::unsorted(vec![var("s"), var("t")], vec![Row::from_slice(&[a, NO_ID])]);
    let right = Bindings::unsorted(vec![var("s"), var("t")], vec![Row::from_slice(&[b, NO_ID])]);
    let result = minus_bindings(left, right);
    // ?s=a != ?s=b → incompatible ⇒ NOT removed.
    assert_eq!(result.rows.len(), 1, "incompatible bound ?s (a != b): left row must be kept");
}
