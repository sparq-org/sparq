// Fixed regression for GitHub #6475: `canonicalize_quads` must be invariant
// under a bijective blank-node rename. The pair below is the shrunk proptest
// counterexample (tests/proptest_canon_determinism.rs, seed
// cc 14c0b82fcdfee2504e7e213e7c440c3f891a8ba23ad553051fd08ab226c0cf2a); it
// reproduced in unmodified rdf-canon 0.15.3. Root cause and patch:
// src/rdfc/SPARQ-PATCHES.md §1 (and §2 for the EXTENDED case below).

use sparq_canon::{canonicalize_nquads, canonicalize_quads, parse_nquads};

const ORIGINAL: &str = "\
_:b4 <http://ex/q> _:b2 _:b3 .
_:b4 <http://ex/p> _:b0 .
_:b0 <http://ex/q> _:b3 _:b2 .
";

// b0->zz0, b2->zz4, b3->zz3, b4->zz5 (bijective across s/o/g positions).
const RENAMED: &str = "\
_:zz5 <http://ex/q> _:zz4 _:zz3 .
_:zz5 <http://ex/p> _:zz0 .
_:zz0 <http://ex/q> _:zz3 _:zz4 .
";

/// Every label permutation of the four blank nodes must canonicalize identically.
const LABELS: [&str; 4] = ["b0", "b2", "b3", "b4"];

#[test]
fn issue_6475_fixed_pair_is_relabel_invariant() {
    let a = canonicalize_quads(&parse_nquads(ORIGINAL).unwrap()).unwrap();
    let b = canonicalize_quads(&parse_nquads(RENAMED).unwrap()).unwrap();
    assert_eq!(a, b, "canonical form leaked input blank-node labels");
    assert_eq!(canonicalize_nquads(ORIGINAL).unwrap(), a);
    assert_eq!(canonicalize_nquads(RENAMED).unwrap(), a);
}

#[test]
fn issue_6475_all_label_permutations_agree() {
    let reference = canonicalize_nquads(ORIGINAL).unwrap();
    let mut perm = [0usize, 1, 2, 3];
    // Heap's algorithm over the 24 permutations of the label pool.
    let mut c = [0usize; 4];
    let check = |perm: &[usize; 4]| {
        let mut text = ORIGINAL.to_string();
        // Two-phase rename so overlapping labels cannot collide.
        for (i, l) in LABELS.iter().enumerate() {
            text = text.replace(&format!("_:{l} "), &format!("_:T{i} "));
        }
        for (i, &p) in perm.iter().enumerate() {
            text = text.replace(&format!("_:T{i} "), &format!("_:{} ", LABELS[p]));
        }
        assert_eq!(
            canonicalize_nquads(&text).unwrap(),
            reference,
            "relabeling {perm:?} changed the canonical form:\n{text}"
        );
    };
    check(&perm);
    let mut i = 0;
    while i < 4 {
        if c[i] < i {
            if i % 2 == 0 {
                perm.swap(0, i);
            } else {
                perm.swap(c[i], i);
            }
            check(&perm);
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
}

// Review follow-up on #6475: ORIGINAL plus two root nodes. `_:b1` relates to `_:b2`
// and `_:b3` with the same related hash, and the two Hash N-Degree Quads paths
// through them are EQUAL. Unpatched HNDQ keeps the first permutation, so its issuer
// (and the c14n4/c14n5 split of the <http://ex/q> quads) followed the input labels.
// `_:b1` and `_:b5` get different N-degree hashes, so the 4.4.3 (5.3) tie-break never
// sees this tie; SPARQ PATCH §2 resolves it inside HNDQ.
const EXTENDED: &str = "\
_:b4 <http://ex/q> _:b2 _:b3 .
_:b4 <http://ex/p> _:b0 .
_:b0 <http://ex/q> _:b3 _:b2 .
_:b1 <http://ex/root0> _:b2 .
_:b1 <http://ex/root0> _:b3 .
_:b5 <http://ex/root0> _:b0 .
_:b5 <http://ex/root0> _:b4 .
";

const EXTENDED_LABELS: [&str; 6] = ["b0", "b1", "b2", "b3", "b4", "b5"];

/// Rename `labels[i]` to `labels[perm[i]]` (two-phase, so overlapping labels cannot collide).
fn relabel(text: &str, labels: &[&str], perm: &[usize]) -> String {
    let mut text = text.to_string();
    for (i, l) in labels.iter().enumerate() {
        text = text.replace(&format!("_:{l} "), &format!("_:T{i} "));
    }
    for (i, &p) in perm.iter().enumerate() {
        text = text.replace(&format!("_:T{i} "), &format!("_:{} ", labels[p]));
    }
    text
}

#[test]
fn equal_hndq_paths_swap_b2_b3_is_relabel_invariant() {
    // Swap b2 <-> b3 only: the reported counterexample.
    let swapped = relabel(EXTENDED, &EXTENDED_LABELS, &[0, 1, 3, 2, 4, 5]);
    assert_eq!(
        canonicalize_nquads(&swapped).unwrap(),
        canonicalize_nquads(EXTENDED).unwrap(),
        "swapping _:b2 and _:b3 changed the canonical form:\n{swapped}"
    );
}

#[test]
fn equal_hndq_paths_all_label_permutations_agree() {
    let reference = canonicalize_nquads(EXTENDED).unwrap();
    let n = EXTENDED_LABELS.len();
    let mut perm: Vec<usize> = (0..n).collect();
    let mut c = vec![0usize; n];
    let check = |perm: &[usize]| {
        let text = relabel(EXTENDED, &EXTENDED_LABELS, perm);
        assert_eq!(
            canonicalize_nquads(&text).unwrap(),
            reference,
            "relabeling {perm:?} changed the canonical form:\n{text}"
        );
    };
    // Heap's algorithm over all 720 permutations of the label pool.
    check(&perm);
    let mut i = 0;
    while i < n {
        if c[i] < i {
            if i % 2 == 0 {
                perm.swap(0, i);
            } else {
                perm.swap(c[i], i);
            }
            check(&perm);
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
}

/// The native RDF 1.2 path (`canonicalize_rdf12`) must give the same bytes as the
/// standard path on triple-term-free input, including every relabeling of both
/// regression datasets, so it needs the same tie-breaks.
#[cfg(feature = "rdf12-triple-terms")]
#[test]
fn rdf12_path_agrees_with_standard_path_on_tie_cases() {
    use sparq_canon::canonicalize_rdf12;

    fn permutations(n: usize) -> Vec<Vec<usize>> {
        if n == 0 {
            return vec![Vec::new()];
        }
        let mut out = Vec::new();
        for p in permutations(n - 1) {
            for at in 0..=p.len() {
                let mut q = p.clone();
                q.insert(at, n - 1);
                out.push(q);
            }
        }
        out
    }

    for (text, labels) in [(ORIGINAL, &LABELS[..]), (EXTENDED, &EXTENDED_LABELS[..])] {
        let reference = canonicalize_nquads(text).unwrap();
        for perm in permutations(labels.len()) {
            let relabeled = relabel(text, labels, &perm);
            let quads = parse_nquads(&relabeled).unwrap();
            assert_eq!(
                canonicalize_rdf12(&quads).unwrap(),
                reference,
                "rdf12 path disagrees with the standard path for relabeling {perm:?}:\n{relabeled}"
            );
            assert_eq!(canonicalize_quads(&quads).unwrap(), reference);
        }
    }
}
