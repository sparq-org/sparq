//! The solution-**multiset** comparator and the `ORDER BY` sort-key-equivalence-class comparator,
//! over the neutral `{var → term}` model. Engine-independent. sq-qcnn.4
//!
//! A SPARQL `SELECT` result is a **bag** of solution mappings (`DISTINCT` is not implicit, so
//! duplicates are significant): the un-ordered comparison is exact **multiset equality**, keyed by the
//! value-canonical form of each term ([`crate::term::canonical_key`]). This subsumes the old
//! cardinality-only cross-check (equal multisets ⇒ equal cardinality, *plus* the values).
//!
//! `ORDER BY` results are compared **up to permutation within each maximal run of sort-key-equal
//! rows**, because SPARQL `ORDER BY` is a *partial* order — rows equal on all sort keys may appear in
//! any relative order across engines. Comparing the full sequence element-for-element would be wrong
//! in general (it is only safe when the sort key is a total order over the projected variables).
//!
//! These are pure comparators over already-normalised results; *wiring* them into the fuzz harness
//! against a live oracle is a separate DAG node (`sq-qcnn.5`).

use crate::json::Solution;
use crate::term::canonical_key;

/// A solution's value-canonical multiset key: the sorted `(var, term-key)` pairs held **structurally**
/// (a `Vec` of pairs, not a delimiter-joined string). Delimiter-joining would be collision-prone — a
/// literal lexical form can contain *any* byte, including a chosen separator, so a differently-shaped
/// solution could forge an identical joined string and read as "equal". A vector of pairs has no
/// separator to forge: two solutions share a key iff they bind the same variables to the same
/// value-canonical terms.
type SolutionKey = Vec<(String, String)>;

/// A sort-variable-projection key: one `Option` per sort variable (`None` marks an unbound sort var, so
/// bound-vs-unbound stays a distinct sort key), held structurally for the same anti-collision reason as
/// [`SolutionKey`]. Drives the `ORDER BY` run partitioning.
type SortKey = Vec<Option<String>>;

fn solution_key(sol: &Solution) -> SolutionKey {
    // `Solution` is a BTreeMap, so iteration is already in sorted-variable order.
    sol.iter()
        .map(|(var, term)| (var.clone(), canonical_key(term)))
        .collect()
}

fn sort_key(sol: &Solution, sort_vars: &[&str]) -> SortKey {
    sort_vars
        .iter()
        .map(|v| sol.get(*v).map(canonical_key))
        .collect()
}

/// Exact **multiset (bag) equality** of two solution sequences, order-insensitive: equal iff they
/// contain the same solutions with the same multiplicities (each solution keyed value-canonically).
pub fn multiset_equal(a: &[Solution], b: &[Solution]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut ka: Vec<SolutionKey> = a.iter().map(solution_key).collect();
    let mut kb: Vec<SolutionKey> = b.iter().map(solution_key).collect();
    ka.sort_unstable();
    kb.sort_unstable();
    ka == kb
}

/// Compare two `ORDER BY` result sequences up to permutation within each sort-key-equivalence class.
///
/// Both sequences are partitioned into maximal runs of rows sharing a `sort_key` (over `sort_vars`);
/// the runs must line up (same count, same order, matching sort keys) and each corresponding run-pair
/// must be **multiset-equal**. With `sort_vars` empty this degrades to [`multiset_equal`].
///
/// Note: with a *total* sort key over all projected variables, each run is a single row and this
/// becomes exact sequence equality (the strict option the harness can force with a tiebreaker).
pub fn order_by_equal(a: &[Solution], b: &[Solution], sort_vars: &[&str]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let runs_a = runs(a, sort_vars);
    let runs_b = runs(b, sort_vars);
    if runs_a.len() != runs_b.len() {
        return false;
    }
    for (&(sa, ea), &(sb, eb)) in runs_a.iter().zip(&runs_b) {
        // Corresponding runs must share the sort key AND be equal as multisets (which also enforces
        // equal run length).
        if sort_key(&a[sa], sort_vars) != sort_key(&b[sb], sort_vars) {
            return false;
        }
        if !multiset_equal(&a[sa..ea], &b[sb..eb]) {
            return false;
        }
    }
    true
}

/// Partition a sequence into maximal `[start, end)` runs of consecutive rows with the same sort key.
fn runs(seq: &[Solution], sort_vars: &[&str]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < seq.len() {
        let key = sort_key(&seq[i], sort_vars);
        let mut j = i + 1;
        while j < seq.len() && sort_key(&seq[j], sort_vars) == key {
            j += 1;
        }
        out.push((i, j));
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Term, XSD_STRING};

    const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";

    fn int(n: &str) -> Term {
        Term::Literal {
            lexical: n.to_string(),
            datatype: XSD_INT.to_string(),
            lang: None,
        }
    }
    fn s(v: &str) -> Term {
        Term::Literal {
            lexical: v.to_string(),
            datatype: XSD_STRING.to_string(),
            lang: None,
        }
    }
    fn sol(pairs: &[(&str, Term)]) -> Solution {
        pairs.iter().map(|(k, t)| (k.to_string(), t.clone())).collect()
    }

    #[test]
    fn multiset_equal_is_order_insensitive_and_value_level() {
        let a = vec![sol(&[("x", int("1"))]), sol(&[("x", int("2"))])];
        let b = vec![sol(&[("x", int("2"))]), sol(&[("x", int("1"))])];
        assert!(multiset_equal(&a, &b), "order must not matter");
        // value-level: same cardinality, different value -> NOT equal (the whole point).
        let c = vec![sol(&[("x", int("1"))]), sol(&[("x", int("3"))])];
        assert!(!multiset_equal(&a, &c));
        // lexical variance of the same value collapses (value regime).
        let d = vec![sol(&[("x", int("01"))]), sol(&[("x", int("2"))])];
        assert!(multiset_equal(&a, &d));
        // multiplicities matter (bag, not set).
        let dup = vec![sol(&[("x", int("1"))]), sol(&[("x", int("1"))])];
        assert!(!multiset_equal(&a, &dup));
        // bound-vs-unbound is a distinct solution.
        let bound = vec![sol(&[("x", int("1")), ("y", s("z"))])];
        let unbound = vec![sol(&[("x", int("1"))])];
        assert!(!multiset_equal(&bound, &unbound));
        // different length.
        assert!(!multiset_equal(&a, &a[..1]));
    }

    #[test]
    fn multiset_equal_is_not_fooled_by_delimiter_bytes_in_a_lexical() {
        // Regression for the old delimiter-joined string key: a literal lexical may contain ANY byte,
        // including the historical record/group/unit separators (\u{1e}/\u{1d}/\u{1f}). These two
        // solutions are genuinely different SHAPES (one variable vs two), yet the single lexical of `a`
        // is crafted so that the old `join`-based key of `a` stringified *identically* to that of `b`.
        // With a structural key they must compare UNEQUAL.
        let key_n = canonical_key(&s("N")); // the value-canonical key `b` would emit for its ?y = "N"
        let forged = format!("M\u{1e}b\u{1d}{key_n}");
        let a = vec![sol(&[("a", s(&forged))])];
        let b = vec![sol(&[("a", s("M")), ("b", s("N"))])];
        // Equal solution COUNT, so this exercises the key comparison rather than the length short-circuit.
        assert_eq!(a.len(), b.len());
        assert!(
            !multiset_equal(&a, &b),
            "a crafted literal lexical must not forge a differently-shaped solution's key"
        );
    }

    #[test]
    fn order_by_equal_permutes_within_ties() {
        // ORDER BY ?k; rows tied on ?k may permute their ?v across engines.
        let a = vec![
            sol(&[("k", int("1")), ("v", s("a"))]),
            sol(&[("k", int("1")), ("v", s("b"))]),
            sol(&[("k", int("2")), ("v", s("c"))]),
        ];
        let b = vec![
            sol(&[("k", int("1")), ("v", s("b"))]),
            sol(&[("k", int("1")), ("v", s("a"))]),
            sol(&[("k", int("2")), ("v", s("c"))]),
        ];
        assert!(order_by_equal(&a, &b, &["k"]), "tie run may permute");
        // but the run CONTENTS must still match as a multiset.
        let c = vec![
            sol(&[("k", int("1")), ("v", s("a"))]),
            sol(&[("k", int("1")), ("v", s("x"))]),
            sol(&[("k", int("2")), ("v", s("c"))]),
        ];
        assert!(!order_by_equal(&a, &c, &["k"]));
        // cross-run reordering is a real order violation -> NOT equal.
        let d = vec![
            sol(&[("k", int("2")), ("v", s("c"))]),
            sol(&[("k", int("1")), ("v", s("a"))]),
            sol(&[("k", int("1")), ("v", s("b"))]),
        ];
        assert!(!order_by_equal(&a, &d, &["k"]));
        // differing lengths.
        assert!(!order_by_equal(&a, &a[..2], &["k"]));
        // empty sort key degrades to multiset equality.
        assert!(order_by_equal(&a, &b, &[]));
    }
}
