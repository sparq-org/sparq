//! Solution-set comparison with blank-node bijection: BAG (multiset) semantics
//! for unordered results, SEQUENCE semantics when the query carries ORDER BY.
//! The same machinery compares graphs (triples-as-rows) for UPDATE tests.

use oxrdf::Term;
use rustc_hash::FxHashMap;

/// A solution row aligned to a shared variable order; `None` = unbound.
pub type Row = Vec<Option<Term>>;

type BnodeMap = FxHashMap<String, String>;

/// Search budget for the multiset-matching backtracker: conformance result
/// sets are small, so hitting this means something is pathological.
const MAX_STEPS: usize = 5_000_000;

/// Whether two rows can be paired under (an extension of) the current blank
/// node bijection. On success the new pairs are appended to `added` and the
/// maps are updated; the caller undoes via [`undo`] on backtrack.
fn row_match(
    exp: &Row,
    act: &Row,
    fwd: &mut BnodeMap,
    rev: &mut BnodeMap,
    added: &mut Vec<String>,
) -> bool {
    if exp.len() != act.len() {
        return false;
    }
    let start = added.len();
    for (e, a) in exp.iter().zip(act) {
        let ok = match (e, a) {
            (None, None) => true,
            (Some(e), Some(a)) => term_match(e, a, fwd, rev, added),
            _ => false,
        };
        if !ok {
            undo(fwd, rev, added, start);
            return false;
        }
    }
    true
}

/// Term equality under (an extension of) the current blank-node bijection,
/// recursing into RDF 1.2 triple terms so their inner blank nodes are mapped
/// too. Mutates the maps; the caller backtracks via [`undo`].
fn term_match(
    e: &Term,
    a: &Term,
    fwd: &mut BnodeMap,
    rev: &mut BnodeMap,
    added: &mut Vec<String>,
) -> bool {
    match (e, a) {
        (Term::BlankNode(eb), Term::BlankNode(ab)) => {
            let (eb, ab) = (eb.as_str(), ab.as_str());
            match (fwd.get(eb), rev.get(ab)) {
                (Some(m), Some(r)) => m == ab && r == eb,
                (None, None) => {
                    fwd.insert(eb.to_string(), ab.to_string());
                    rev.insert(ab.to_string(), eb.to_string());
                    added.push(eb.to_string());
                    true
                }
                _ => false,
            }
        }
        (Term::Triple(et), Term::Triple(at)) => {
            et.predicate == at.predicate
                && term_match(
                    &Term::from(et.subject.clone()),
                    &Term::from(at.subject.clone()),
                    fwd,
                    rev,
                    added,
                )
                && term_match(&et.object, &at.object, fwd, rev, added)
        }
        _ => e == a,
    }
}

fn undo(fwd: &mut BnodeMap, rev: &mut BnodeMap, added: &mut Vec<String>, to: usize) {
    while added.len() > to {
        let e = added.pop().unwrap();
        if let Some(a) = fwd.remove(&e) {
            rev.remove(&a);
        }
    }
}

/// Multiset (or, when `ordered`, sequence) equality of two row collections
/// under a single global blank-node bijection.
pub fn rows_equal(expected: &[Row], actual: &[Row], ordered: bool) -> Result<bool, String> {
    if expected.len() != actual.len() {
        return Ok(false);
    }
    let mut fwd = BnodeMap::default();
    let mut rev = BnodeMap::default();
    let mut added = Vec::new();
    if ordered {
        // Pairing is fixed, so within-row bnode choices are forced: no backtracking.
        for (e, a) in expected.iter().zip(actual) {
            if !row_match(e, a, &mut fwd, &mut rev, &mut added) {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let mut used = vec![false; actual.len()];
    let mut steps = 0usize;
    search(
        0, expected, actual, &mut used, &mut fwd, &mut rev, &mut steps,
    )
}

fn search(
    i: usize,
    expected: &[Row],
    actual: &[Row],
    used: &mut [bool],
    fwd: &mut BnodeMap,
    rev: &mut BnodeMap,
    steps: &mut usize,
) -> Result<bool, String> {
    if i == expected.len() {
        return Ok(true);
    }
    let mut added = Vec::new();
    for j in 0..actual.len() {
        if used[j] {
            continue;
        }
        *steps += 1;
        if *steps > MAX_STEPS {
            return Err("comparison budget exceeded".into());
        }
        added.clear();
        if row_match(&expected[i], &actual[j], fwd, rev, &mut added) {
            used[j] = true;
            if search(i + 1, expected, actual, used, fwd, rev, steps)? {
                return Ok(true);
            }
            used[j] = false;
            undo(fwd, rev, &mut added, 0);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode};

    fn iri(s: &str) -> Option<Term> {
        Some(Term::NamedNode(
            NamedNode::new(format!("http://x/{s}")).unwrap(),
        ))
    }
    fn bn(s: &str) -> Option<Term> {
        Some(Term::BlankNode(BlankNode::new(s).unwrap()))
    }
    fn lit(s: &str) -> Option<Term> {
        Some(Term::Literal(Literal::new_simple_literal(s)))
    }

    #[test]
    fn bag_semantics_ignore_order() {
        let a = vec![vec![iri("a")], vec![iri("b")]];
        let b = vec![vec![iri("b")], vec![iri("a")]];
        assert!(rows_equal(&a, &b, false).unwrap());
        assert!(!rows_equal(&a, &b, true).unwrap());
    }

    #[test]
    fn multiset_counts_duplicates() {
        let a = vec![vec![lit("x")], vec![lit("x")]];
        let b = vec![vec![lit("x")], vec![lit("y")]];
        assert!(!rows_equal(&a, &b, false).unwrap());
    }

    #[test]
    fn bnode_bijection_consistent() {
        // _:1/_:2 in expected must map one-to-one onto _:a/_:b.
        let exp = vec![vec![bn("b1"), bn("b1")], vec![bn("b2"), lit("v")]];
        let act = vec![vec![bn("x"), bn("x")], vec![bn("y"), lit("v")]];
        assert!(rows_equal(&exp, &act, false).unwrap());
        // _:1 used for two *different* actual bnodes must fail.
        let act_bad = vec![vec![bn("x"), bn("y")], vec![bn("z"), lit("v")]];
        assert!(!rows_equal(&exp, &act_bad, false).unwrap());
    }

    #[test]
    fn unbound_matters() {
        let a = vec![vec![iri("a"), None]];
        let b = vec![vec![iri("a"), lit("x")]];
        assert!(!rows_equal(&a, &b, false).unwrap());
    }
}
