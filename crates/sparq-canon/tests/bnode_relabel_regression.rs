// Fixed regression for GitHub #6475: `canonicalize_quads` must be invariant
// under a bijective blank-node rename. The pair below is the shrunk proptest
// counterexample (tests/proptest_canon_determinism.rs, seed
// cc 14c0b82fcdfee2504e7e213e7c440c3f891a8ba23ad553051fd08ab226c0cf2a); it
// reproduced in unmodified rdf-canon 0.15.3. Root cause and patch:
// vendor/rdf-canon/SPARQ-PATCHES.md §1.

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
