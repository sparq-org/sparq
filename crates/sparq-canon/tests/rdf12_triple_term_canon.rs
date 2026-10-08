//! Tests for the OPT-IN, NON-STANDARD `rdf12-triple-terms` profile (sq-hslb).
//!
//! Only compiled when the feature is on. The headline invariants:
//!  1. **Standard-path agreement** — on triple-term-free input the v2 profile is
//!     byte-identical to the standard `rdf-canon`-backed path (the strongest
//!     correctness anchor; run over every W3C suite eval vector).
//!  2. **Ground triple-term datasets** — order-invariant, isomorphism-stable.
//!  3. **Nested-bnode triple terms** — a blank node inside a triple-term object
//!     (and deeper) is relabelled by the HNDQ descent; bnode-label and
//!     triple-order invariance hold, including bnodes shared between a top-level
//!     position and a triple-term-internal position. Includes a case the
//!     standard path rejects with `TripleTerm` that now canonicalizes.
#![cfg(feature = "rdf12-triple-terms")]

use oxrdf::{BlankNode, GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term, Triple};
use std::path::{Path, PathBuf};

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rdf-canon-testdata")
}

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn bn(s: &str) -> BlankNode {
    BlankNode::new(s).unwrap()
}

/// Triple-term object `<<( s p o )>>`.
fn tt(s: NamedOrBlankNode, p: NamedNode, o: Term) -> Term {
    Term::Triple(Box::new(Triple::new(s, p, o)))
}

// ---------------------------------------------------------------------------
// 1) Standard-path agreement on triple-term-free input (the W3C suite).
// ---------------------------------------------------------------------------

fn load_quads(path: &Path) -> Vec<Quad> {
    let bytes = std::fs::read(path).unwrap();
    oxttl::NQuadsParser::new()
        .for_slice(&bytes)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// Every default-SHA-256 eval `.nq` input under the suite, canonicalized by both
/// the standard path and the v2 profile, must agree byte-for-byte. This proves
/// the native HNDQ matches `rdf-canon` on the shared (triple-term-free) subset.
#[test]
fn v2_agrees_with_standard_on_suite_inputs() {
    let dir = testdata().join("rdfc10");
    let mut compared = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("-in.nq"))
                .unwrap_or(false)
        })
        .collect();
    entries.sort();

    for path in entries {
        let quads = load_quads(&path);
        // Standard path: skip the few suite inputs the standard path itself
        // rejects (poison graphs hit the HNDQ limit). We only compare where the
        // standard path succeeds — that is exactly the shared subset.
        let std_out = match sparq_canon::canonicalize(&quads) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let v2_out = sparq_canon::canonicalize_rdf12(&quads)
            .unwrap_or_else(|e| panic!("v2 failed on {:?}: {e}", path));
        if std_out != v2_out {
            mismatches.push(format!(
                "{:?}\n  std: {:?}\n  v2:  {:?}",
                path.file_name().unwrap(),
                std_out,
                v2_out
            ));
        }
        compared += 1;
    }
    assert!(
        compared >= 60,
        "expected to compare most suite inputs, got {compared}"
    );
    assert!(
        mismatches.is_empty(),
        "{} suite inputs disagree between standard and v2:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

/// The single-graph v2 API agrees with the standard single-graph API on every
/// default-graph-only, triple-term-free suite eval input.
#[test]
fn v2_single_graph_agrees_with_standard() {
    let dir = testdata().join("rdfc10");
    let mut compared = 0usize;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("-in.nq"))
                .unwrap_or(false)
        })
        .collect();
    entries.sort();
    for path in entries {
        let quads = load_quads(&path);
        if !quads
            .iter()
            .all(|q| q.graph_name == GraphName::DefaultGraph)
        {
            continue;
        }
        let triples: Vec<Triple> = quads
            .iter()
            .map(|q| Triple::new(q.subject.clone(), q.predicate.clone(), q.object.clone()))
            .collect();
        let std_out = match sparq_canon::canonicalize_triples(&triples) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let v2_out = sparq_canon::canonicalize_triples_rdf12(&triples).unwrap();
        assert_eq!(
            std_out.lines,
            v2_out.lines,
            "single-graph lines disagree on {:?}",
            path.file_name().unwrap()
        );
        assert_eq!(
            std_out.triples,
            v2_out.triples,
            "single-graph triples disagree on {:?}",
            path.file_name().unwrap()
        );
        compared += 1;
    }
    assert!(
        compared >= 30,
        "expected most eval inputs default-graph, got {compared}"
    );
}

// ---------------------------------------------------------------------------
// 2) Ground (blank-free) triple-term datasets.
// ---------------------------------------------------------------------------

/// A ground triple-term dataset (no blank nodes anywhere) is order-invariant and
/// canonicalizes to the same bytes regardless of input quad order — the common
/// credential/VC case.
#[test]
fn ground_triple_term_is_order_invariant() {
    let inner = |o: &str| -> Term {
        tt(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::NamedNode(iri(o)),
        )
    };
    let t1 = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        inner("http://ex/o1"),
    );
    let t2 = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/b")),
        iri("http://ex/says"),
        inner("http://ex/o2"),
    );
    let forward = sparq_canon::canonicalize_triples_rdf12(&[t1.clone(), t2.clone()]).unwrap();
    let reverse = sparq_canon::canonicalize_triples_rdf12(&[t2, t1]).unwrap();
    assert_eq!(
        forward.lines, reverse.lines,
        "ground TT must be order-invariant"
    );
    // Sanity: triple-term token form is present in the canonical output.
    assert!(
        forward.lines.iter().any(|l| l.contains("<<(")),
        "expected canonical triple-term token form: {:?}",
        forward.lines
    );
}

/// A triple term whose object is itself a triple term (ground, deep nesting)
/// canonicalizes deterministically and preserves the nested token form.
#[test]
fn ground_deep_nested_triple_term() {
    let deepest = tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/x")),
        iri("http://ex/p"),
        Term::Literal(Literal::new_simple_literal("v")),
    );
    let mid = tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/y")),
        iri("http://ex/q"),
        deepest,
    );
    let t = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        mid,
    );
    let c = sparq_canon::canonicalize_triples_rdf12(std::slice::from_ref(&t)).unwrap();
    assert_eq!(c.lines.len(), 1);
    // Nested `<<( ... <<( ... )>> )>>` round-trips back to the same term.
    assert_eq!(c.triples[0], t, "deep ground TT must round-trip");
}

// ---------------------------------------------------------------------------
// 3) Nested-blank-node triple terms — the headline HNDQ-descent deliverable.
// ---------------------------------------------------------------------------

/// The case the OLD code rejected with `TripleTerm`: a blank node nested inside
/// a triple-term object now canonicalizes, and is relabelled to `c14nN`.
#[test]
fn nested_bnode_now_canonicalizes() {
    let inner = tt(
        NamedOrBlankNode::BlankNode(bn("nested")),
        iri("http://ex/p"),
        Term::Literal(Literal::new_simple_literal("v")),
    );
    let t = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/asserts"),
        inner,
    );
    // Standard path still rejects.
    assert!(matches!(
        sparq_canon::canonicalize_triples(std::slice::from_ref(&t)),
        Err(sparq_canon::CanonError::TripleTerm)
    ));
    // v2 profile canonicalizes and relabels the nested bnode.
    let c = sparq_canon::canonicalize_triples_rdf12(&[t]).unwrap();
    assert_eq!(c.lines.len(), 1);
    assert!(
        c.lines[0].contains("_:c14n0"),
        "nested bnode must be relabelled to c14n0: {}",
        c.lines[0]
    );
}

/// Bnode-label invariance + triple-order invariance for nested-bnode triple
/// terms: two isomorphic graphs differing only by input bnode labels and quad
/// order canonicalize byte-identically, including a bnode shared between a
/// top-level subject and a triple-term-internal position.
#[test]
fn nested_bnode_isomorphism() {
    // Graph shape:
    //   _:shared  ex:says  <<( _:shared ex:p _:other )>>
    //   _:other   ex:t     "leaf"
    // _:shared appears both at top level (subject) AND inside the triple term.
    let build = |shared: &str, other: &str, swap: bool| -> Vec<Triple> {
        let inner = tt(
            NamedOrBlankNode::BlankNode(bn(shared)),
            iri("http://ex/p"),
            Term::BlankNode(bn(other)),
        );
        let t1 = Triple::new(
            NamedOrBlankNode::BlankNode(bn(shared)),
            iri("http://ex/says"),
            inner,
        );
        let t2 = Triple::new(
            NamedOrBlankNode::BlankNode(bn(other)),
            iri("http://ex/t"),
            Term::Literal(Literal::new_simple_literal("leaf")),
        );
        if swap {
            vec![t2, t1]
        } else {
            vec![t1, t2]
        }
    };
    let a = sparq_canon::canonicalize_triples_rdf12(&build("shared", "other", false)).unwrap();
    let b = sparq_canon::canonicalize_triples_rdf12(&build("xxx", "yyy", true)).unwrap();
    assert_eq!(
        a.lines, b.lines,
        "isomorphic nested-bnode graphs must canonicalize identically\n a={:?}\n b={:?}",
        a.lines, b.lines
    );
    // The shared bnode must resolve to one canonical label used in both its
    // top-level and triple-term-internal occurrences.
    assert!(a.lines.iter().any(|l| l.contains("<<(")));
}

/// Distinguishing case: two graphs that are NOT isomorphic (a nested bnode is
/// connected differently) must canonicalize to DIFFERENT output, so the descent
/// is not collapsing distinct structure.
#[test]
fn nested_bnode_distinguishes_non_isomorphic() {
    // G1: _:a ex:says <<( _:a ex:p _:b )>> ;  _:b ex:t "x"
    let g1 = {
        let inner = tt(
            NamedOrBlankNode::BlankNode(bn("a")),
            iri("http://ex/p"),
            Term::BlankNode(bn("b")),
        );
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("a")),
                iri("http://ex/says"),
                inner,
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("b")),
                iri("http://ex/t"),
                Term::Literal(Literal::new_simple_literal("x")),
            ),
        ]
    };
    // G2: _:a ex:says <<( _:b ex:p _:b )>> ;  _:b ex:t "x"  (subject of inner differs)
    let g2 = {
        let inner = tt(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/p"),
            Term::BlankNode(bn("b")),
        );
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("a")),
                iri("http://ex/says"),
                inner,
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("b")),
                iri("http://ex/t"),
                Term::Literal(Literal::new_simple_literal("x")),
            ),
        ]
    };
    let c1 = sparq_canon::canonicalize_triples_rdf12(&g1).unwrap();
    let c2 = sparq_canon::canonicalize_triples_rdf12(&g2).unwrap();
    assert_ne!(c1.lines, c2.lines, "non-isomorphic graphs must differ");
}

/// Dataset-level (named-graph) entry point with a nested bnode triple term.
#[test]
fn dataset_named_graph_nested_bnode() {
    let inner = tt(
        NamedOrBlankNode::BlankNode(bn("n")),
        iri("http://ex/p"),
        Term::Literal(Literal::new_simple_literal("v")),
    );
    let q = Quad::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/asserts"),
        inner,
        GraphName::NamedNode(iri("http://ex/g")),
    );
    let out = sparq_canon::canonicalize_rdf12(std::slice::from_ref(&q)).unwrap();
    assert!(out.contains("<http://ex/g>"), "graph name preserved: {out}");
    assert!(out.contains("_:c14n0"), "nested bnode relabelled: {out}");
    let map = sparq_canon::issue_dataset_rdf12(&[q]).unwrap();
    assert_eq!(map.get("n").map(String::as_str), Some("c14n0"));
}

/// Directional language-tagged string inside a triple term renders with the
/// canonical RDF-1.2 `@lang--dir` token form (single-sourced via oxrdf Display).
#[test]
fn directional_language_string_in_triple_term() {
    let dir_lit =
        Literal::new_directional_language_tagged_literal("שלום", "he", oxrdf::BaseDirection::Rtl)
            .unwrap();
    let inner = tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::Literal(dir_lit),
    );
    let t = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        inner,
    );
    let c = sparq_canon::canonicalize_triples_rdf12(&[t]).unwrap();
    assert!(
        c.lines[0].contains("@he--rtl"),
        "directional language token form expected: {}",
        c.lines[0]
    );
}

// ---------------------------------------------------------------------------
// 4) Hash-profile parity: `*_with::<D: Digest>` (SHA-384) + back-compat guard
//    (sq-5i1d). The non-generic default stays byte-identical to today; SHA-384
//    is purely additive.
// ---------------------------------------------------------------------------

use sha2::{Sha256, Sha384};

/// A symmetric two-bnode cycle whose c14n order is decided by the n-degree hash
/// (the two bnodes share a first-degree hash), so the choice of hash function is
/// observable in the relabelling. Shared by the parity tests below.
fn symmetric_cycle_dataset() -> Vec<Quad> {
    let mk = |s: &str, o: &str| {
        Quad::new(
            NamedOrBlankNode::BlankNode(bn(s)),
            iri("http://ex/p"),
            Term::BlankNode(bn(o)),
            GraphName::DefaultGraph,
        )
    };
    vec![
        mk("a", "b"),
        mk("b", "a"),
        Quad::new(
            NamedOrBlankNode::BlankNode(bn("a")),
            iri("http://ex/t"),
            Term::Literal(Literal::new_simple_literal("x")),
            GraphName::DefaultGraph,
        ),
        Quad::new(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/u"),
            Term::Literal(Literal::new_simple_literal("y")),
            GraphName::DefaultGraph,
        ),
    ]
}

/// BACK-COMPAT GUARD: the non-generic default (`canonicalize_rdf12`) must be
/// byte-identical to the explicit SHA-256 profile (`canonicalize_rdf12_with::<
/// Sha256>`) across every entry point. This is the load-bearing back-compat
/// invariant: the SHA-384 variant is purely additive.
#[test]
fn default_is_sha256_byte_identical() {
    let ds = symmetric_cycle_dataset();
    assert_eq!(
        sparq_canon::canonicalize_rdf12(&ds).unwrap(),
        sparq_canon::canonicalize_rdf12_with::<Sha256>(&ds).unwrap(),
        "non-generic default must equal the explicit SHA-256 profile"
    );
    assert_eq!(
        sparq_canon::issue_dataset_rdf12(&ds).unwrap(),
        sparq_canon::issue_dataset_rdf12_with::<Sha256>(&ds).unwrap(),
        "issuer maps must match"
    );
    let triples: Vec<Triple> = ds
        .iter()
        .map(|q| Triple::new(q.subject.clone(), q.predicate.clone(), q.object.clone()))
        .collect();
    let g_default = sparq_canon::canonicalize_triples_rdf12(&triples).unwrap();
    let g_sha256 = sparq_canon::canonicalize_triples_rdf12_with::<Sha256>(&triples).unwrap();
    assert_eq!(g_default.lines, g_sha256.lines);
    assert_eq!(g_default.triples, g_sha256.triples);
}

/// On a ground (blank-node-free) dataset no hash feeds into a relabelling, so the
/// canonical OUTPUT is identical under any hash. This guards against an
/// accidental hash leak into the serialized line text: the hash only ever shows
/// up in intermediate labelling, never in the output bytes.
#[test]
fn sha384_ground_output_equals_sha256() {
    let inner = tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::NamedNode(iri("http://ex/o")),
    );
    let t = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        inner,
    );
    let sha256 =
        sparq_canon::canonicalize_triples_rdf12_with::<Sha256>(std::slice::from_ref(&t)).unwrap();
    let sha384 =
        sparq_canon::canonicalize_triples_rdf12_with::<Sha384>(std::slice::from_ref(&t)).unwrap();
    assert_eq!(
        sha256.lines, sha384.lines,
        "ground (bnode-free) output must not depend on the hash function"
    );
}

/// The SHA-384 profile is DETERMINISTIC: the same dataset canonicalizes to the
/// same bytes on repeated runs.
#[test]
fn sha384_is_deterministic() {
    let ds = symmetric_cycle_dataset();
    let a = sparq_canon::canonicalize_rdf12_with::<Sha384>(&ds).unwrap();
    let b = sparq_canon::canonicalize_rdf12_with::<Sha384>(&ds).unwrap();
    assert_eq!(a, b, "SHA-384 profile must be deterministic");
}

/// The SHA-384 profile is ISOMORPHISM-STABLE under that hash: two isomorphic
/// graphs (relabelled bnodes + permuted quad order) canonicalize byte-identically
/// — including a nested-bnode triple term.
#[test]
fn sha384_isomorphism_stable() {
    let build = |shared: &str, other: &str, swap: bool| -> Vec<Triple> {
        let inner = tt(
            NamedOrBlankNode::BlankNode(bn(shared)),
            iri("http://ex/p"),
            Term::BlankNode(bn(other)),
        );
        let t1 = Triple::new(
            NamedOrBlankNode::BlankNode(bn(shared)),
            iri("http://ex/says"),
            inner,
        );
        let t2 = Triple::new(
            NamedOrBlankNode::BlankNode(bn(other)),
            iri("http://ex/t"),
            Term::Literal(Literal::new_simple_literal("leaf")),
        );
        if swap {
            vec![t2, t1]
        } else {
            vec![t1, t2]
        }
    };
    let a =
        sparq_canon::canonicalize_triples_rdf12_with::<Sha384>(&build("shared", "other", false))
            .unwrap();
    let b =
        sparq_canon::canonicalize_triples_rdf12_with::<Sha384>(&build("xxx", "yyy", true)).unwrap();
    assert_eq!(
        a.lines, b.lines,
        "SHA-384 profile must be isomorphism-stable\n a={:?}\n b={:?}",
        a.lines, b.lines
    );
    assert!(a.lines.iter().any(|l| l.contains("<<(")));
}

/// SHA-384 still DISTINGUISHES non-isomorphic graphs (the descent isn't
/// collapsing distinct structure under the wider hash).
#[test]
fn sha384_distinguishes_non_isomorphic() {
    let g1 = {
        let inner = tt(
            NamedOrBlankNode::BlankNode(bn("a")),
            iri("http://ex/p"),
            Term::BlankNode(bn("b")),
        );
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("a")),
                iri("http://ex/says"),
                inner,
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("b")),
                iri("http://ex/t"),
                Term::Literal(Literal::new_simple_literal("x")),
            ),
        ]
    };
    let g2 = {
        let inner = tt(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/p"),
            Term::BlankNode(bn("b")),
        );
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("a")),
                iri("http://ex/says"),
                inner,
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("b")),
                iri("http://ex/t"),
                Term::Literal(Literal::new_simple_literal("x")),
            ),
        ]
    };
    let c1 = sparq_canon::canonicalize_triples_rdf12_with::<Sha384>(&g1).unwrap();
    let c2 = sparq_canon::canonicalize_triples_rdf12_with::<Sha384>(&g2).unwrap();
    assert_ne!(
        c1.lines, c2.lines,
        "non-isomorphic graphs must differ under SHA-384"
    );
}

// ---------------------------------------------------------------------------
// 5) Nested-bnode DISTINGUISHING-POWER regression vectors (sq-mu1cd, from the
//    adversarial soundness audit of #933/#960 — see the comment on sq-63g0).
//
//    The audit found the canon SOUND (0 confirmed / 5 refuted): the HNDQ
//    position-marker collapse the reviewers flagged (all triple-term-internal
//    bnodes hashed with `Position::Object` + the OUTER predicate, with no inner
//    subject/object/depth sub-discriminator — rdf12.rs `hash_n_degree_quads` /
//    `hash_related_blank_node`) is a real LOSS of distinguishing power in the
//    marker, but NOT a soundness defect, because the final serialization
//    re-renders the actual `Term::Triple` structure with c14n labels and the
//    issuer map still separates genuinely-distinct bnodes via the first-degree
//    descent. These are the audit's SHARPER non-isomorphic vectors landed as
//    permanent regression tests: a future refactor of `hash_n_degree_quads` /
//    `hash_related_blank_node` that DID introduce a false-equal would be caught
//    here. [OPUS-4.8]
//
//    CAUTION (the audit's own): one hand-authored audit pair had a WRONG
//    isomorphism premise (it was non-isomorphic, not isomorphic), so its
//    "failure" was a TEST bug, not a canon bug. To make this class of mistake
//    impossible, every "must DIFFER" vector below FIRST proves its pair is
//    genuinely non-isomorphic with the independent brute-force oracle
//    `is_isomorphic` (a structural bnode-bijection search that does NOT use the
//    canon under test), then asserts the canon distinguishes it. The oracle is
//    sanity-checked against a known automorphism in `oracle_detects_automorphism`.
// ---------------------------------------------------------------------------

use std::collections::{BTreeSet, HashMap};

fn lit(s: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(s))
}

/// Brute-force RDF-1.2 isomorphism oracle — INDEPENDENT of the canon under test.
/// Two single-graph triple slices are isomorphic iff some bijection of `g1`'s
/// blank nodes onto `g2`'s (recursing through triple-term subject/object) makes
/// their triple SETS equal (IRIs / literals are fixed). Worst case is `n!` in the
/// shared bnode count; all vectors here have ≤ 3 bnodes, so this is trivial. This
/// is the load-bearing premise-checker for every `must DIFFER` assertion below —
/// we never assert NE on a pair we have not first proven non-isomorphic.
fn is_isomorphic(g1: &[Triple], g2: &[Triple]) -> bool {
    fn collect_subject(s: &NamedOrBlankNode, out: &mut BTreeSet<String>) {
        if let NamedOrBlankNode::BlankNode(b) = s {
            out.insert(b.as_str().to_string());
        }
    }
    fn collect_term(t: &Term, out: &mut BTreeSet<String>) {
        match t {
            Term::BlankNode(b) => {
                out.insert(b.as_str().to_string());
            }
            Term::Triple(tr) => {
                collect_subject(&tr.subject, out);
                collect_term(&tr.object, out);
            }
            _ => {}
        }
    }
    fn graph_bnodes(g: &[Triple]) -> Vec<String> {
        let mut s = BTreeSet::new();
        for t in g {
            collect_subject(&t.subject, &mut s);
            collect_term(&t.object, &mut s);
        }
        s.into_iter().collect()
    }
    fn remap_subject(s: &NamedOrBlankNode, m: &HashMap<String, String>) -> NamedOrBlankNode {
        match s {
            NamedOrBlankNode::BlankNode(b) => {
                NamedOrBlankNode::BlankNode(bn(m.get(b.as_str()).unwrap()))
            }
            other => other.clone(),
        }
    }
    fn remap_term(t: &Term, m: &HashMap<String, String>) -> Term {
        match t {
            Term::BlankNode(b) => Term::BlankNode(bn(m.get(b.as_str()).unwrap())),
            Term::Triple(tr) => Term::Triple(Box::new(Triple::new(
                remap_subject(&tr.subject, m),
                tr.predicate.clone(),
                remap_term(&tr.object, m),
            ))),
            other => other.clone(),
        }
    }
    // Canonical line via oxrdf-0.3 Display (the same token form the canon uses),
    // optionally under a bnode relabelling.
    fn lines(g: &[Triple], m: Option<&HashMap<String, String>>) -> BTreeSet<String> {
        g.iter()
            .map(|t| {
                let nt = match m {
                    Some(m) => Triple::new(
                        remap_subject(&t.subject, m),
                        t.predicate.clone(),
                        remap_term(&t.object, m),
                    ),
                    None => t.clone(),
                };
                format!("{} {} {} .", nt.subject, nt.predicate, nt.object)
            })
            .collect()
    }
    fn perms(items: &[String]) -> Vec<Vec<String>> {
        if items.is_empty() {
            return vec![vec![]];
        }
        let mut out = Vec::new();
        for i in 0..items.len() {
            let mut rest = items.to_vec();
            let head = rest.remove(i);
            for mut p in perms(&rest) {
                p.insert(0, head.clone());
                out.push(p);
            }
        }
        out
    }

    let b1 = graph_bnodes(g1);
    let b2 = graph_bnodes(g2);
    if b1.len() != b2.len() {
        return false;
    }
    let target = lines(g2, None);
    if b1.is_empty() {
        return lines(g1, None) == target;
    }
    for perm in perms(&b2) {
        let mut m = HashMap::new();
        for (a, c) in b1.iter().zip(perm.iter()) {
            m.insert(a.clone(), c.clone());
        }
        if lines(g1, Some(&m)) == target {
            return true;
        }
    }
    false
}

/// Sanity: the oracle is not trivially returning `false`. A genuine automorphism
/// (the symmetric inner swap, where the two inner leaf bnodes carry the SAME
/// anchor predicate) must be reported isomorphic — this is the case the canon
/// correctly canonicalizes to EQUAL output, and it guards against the oracle
/// silently rejecting everything (which would make the NE premises vacuous).
#[test]
fn oracle_detects_automorphism() {
    let sym = |swap: bool| -> Vec<Triple> {
        let (s, o) = if swap { ("c", "b") } else { ("b", "c") };
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("a")),
                iri("http://ex/p"),
                tt(
                    NamedOrBlankNode::BlankNode(bn(s)),
                    iri("http://ex/q"),
                    Term::BlankNode(bn(o)),
                ),
            ),
            // SAME anchor predicate on both inner leaves => a real automorphism.
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("b")),
                iri("http://ex/t"),
                lit("x"),
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("c")),
                iri("http://ex/t"),
                lit("x"),
            ),
        ]
    };
    assert!(
        is_isomorphic(&sym(false), &sym(true)),
        "oracle must detect the symmetric-swap automorphism"
    );
    // And the canon agrees: the automorphic pair canonicalizes to EQUAL output.
    let a = sparq_canon::canonicalize_triples_rdf12(&sym(false)).unwrap();
    let b = sparq_canon::canonicalize_triples_rdf12(&sym(true)).unwrap();
    assert_eq!(a.lines, b.lines, "automorphic pair must canonicalize equal");
}

/// Vector 1 — asymmetric inner swap. The inner subject/object carries the ONLY
/// distinguishing role: `_:b`/`_:c` are pinned to DISTINCT leaf predicates
/// (`ex:t1` / `ex:t2`), so swapping which sits in the inner-subject vs the
/// inner-object slot is NOT an automorphism. Brute-force oracle: NON-isomorphic.
/// The canon must DIFFER (the inner subject/object position must reach the label).
#[test]
fn vec1_asymmetric_inner_swap_differs() {
    let g = |swap: bool| -> Vec<Triple> {
        let (s, o) = if swap { ("c", "b") } else { ("b", "c") };
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("a")),
                iri("http://ex/p"),
                tt(
                    NamedOrBlankNode::BlankNode(bn(s)),
                    iri("http://ex/q"),
                    Term::BlankNode(bn(o)),
                ),
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("b")),
                iri("http://ex/t1"),
                lit("x"),
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("c")),
                iri("http://ex/t2"),
                lit("x"),
            ),
        ]
    };
    let g1 = g(false);
    let g2 = g(true);
    assert!(
        !is_isomorphic(&g1, &g2),
        "premise: asymmetric inner swap must be non-isomorphic"
    );
    let c1 = sparq_canon::canonicalize_triples_rdf12(&g1).unwrap();
    let c2 = sparq_canon::canonicalize_triples_rdf12(&g2).unwrap();
    assert_ne!(
        c1.lines, c2.lines,
        "asymmetric inner swap must canonicalize differently"
    );
}

/// Vector 2 — inner-SUBJECT vs inner-OBJECT leaf bnode. `_:a ex:p <<( _:b ex:q
/// "lit" )>>` (bnode in the inner SUBJECT, literal object) vs `_:x ex:p <<( ex:s
/// ex:q _:y )>>` (IRI subject, bnode in the inner OBJECT). Different ground
/// skeleton — even bnode-erased these are not the same graph — so NON-isomorphic.
/// The canon must DIFFER. Exercises the exact position the audit flagged.
#[test]
fn vec2_inner_subject_vs_inner_object_leaf_differs() {
    let g1 = vec![Triple::new(
        NamedOrBlankNode::BlankNode(bn("a")),
        iri("http://ex/p"),
        tt(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/q"),
            lit("lit"),
        ),
    )];
    let g2 = vec![Triple::new(
        NamedOrBlankNode::BlankNode(bn("x")),
        iri("http://ex/p"),
        tt(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/q"),
            Term::BlankNode(bn("y")),
        ),
    )];
    assert!(
        !is_isomorphic(&g1, &g2),
        "premise: inner-subject vs inner-object leaf must be non-isomorphic"
    );
    let c1 = sparq_canon::canonicalize_triples_rdf12(&g1).unwrap();
    let c2 = sparq_canon::canonicalize_triples_rdf12(&g2).unwrap();
    assert_ne!(
        c1.lines, c2.lines,
        "inner-subject vs inner-object leaf bnode must canonicalize differently"
    );
}

/// Vector 3 — depth-1 vs depth-2 nesting of the SAME bnode set `{_:a,_:b,_:c}`.
/// depth-1: `_:a ex:p <<( _:b ex:q _:c )>>`. depth-2: `_:a ex:p <<( _:b ex:q
/// <<( _:c ex:r "z" )>> )>>` — `_:c` now sits one level deeper, inside a second
/// triple term. Different nesting structure => NON-isomorphic. The canon must
/// DIFFER (depth must not collapse under the object marker).
#[test]
fn vec3_depth1_vs_depth2_nesting_differs() {
    let depth1 = vec![Triple::new(
        NamedOrBlankNode::BlankNode(bn("a")),
        iri("http://ex/p"),
        tt(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/q"),
            Term::BlankNode(bn("c")),
        ),
    )];
    let depth2 = vec![Triple::new(
        NamedOrBlankNode::BlankNode(bn("a")),
        iri("http://ex/p"),
        tt(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/q"),
            tt(
                NamedOrBlankNode::BlankNode(bn("c")),
                iri("http://ex/r"),
                lit("z"),
            ),
        ),
    )];
    assert!(
        !is_isomorphic(&depth1, &depth2),
        "premise: depth-1 vs depth-2 nesting must be non-isomorphic"
    );
    let c1 = sparq_canon::canonicalize_triples_rdf12(&depth1).unwrap();
    let c2 = sparq_canon::canonicalize_triples_rdf12(&depth2).unwrap();
    assert_ne!(
        c1.lines, c2.lines,
        "depth-1 vs depth-2 nesting of the same bnode set must canonicalize differently"
    );
}

/// Vector 4 — anchored inner swap with DISTINCT anchor predicates (`ex:A` /
/// `ex:B`), so there is NO automorphism: `_:b` and `_:c` are individually
/// pinned. Swapping them across the inner subject/object slot is non-isomorphic.
/// Also asserts the load-bearing audit observation (M2): the issuer map assigns
/// 3 DISTINCT c14n labels to `{_:a,_:b,_:c}`, i.e. the descent separates all
/// three genuinely-distinct bnodes even though the inner two share the object
/// position marker. The canon must DIFFER.
#[test]
fn vec4_anchored_inner_swap_differs_and_three_distinct_labels() {
    let g = |swap: bool| -> Vec<Triple> {
        let (s, o) = if swap { ("c", "b") } else { ("b", "c") };
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("a")),
                iri("http://ex/p"),
                tt(
                    NamedOrBlankNode::BlankNode(bn(s)),
                    iri("http://ex/q"),
                    Term::BlankNode(bn(o)),
                ),
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("b")),
                iri("http://ex/A"),
                lit("1"),
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(bn("c")),
                iri("http://ex/B"),
                lit("2"),
            ),
        ]
    };
    let g1 = g(false);
    let g2 = g(true);
    assert!(
        !is_isomorphic(&g1, &g2),
        "premise: anchored inner swap (distinct anchors) must be non-isomorphic"
    );
    let c1 = sparq_canon::canonicalize_triples_rdf12(&g1).unwrap();
    let c2 = sparq_canon::canonicalize_triples_rdf12(&g2).unwrap();
    assert_ne!(
        c1.lines, c2.lines,
        "anchored inner swap must canonicalize differently"
    );
    // (M2) issuer map separates ALL THREE distinct bnodes into 3 distinct labels.
    let map = sparq_canon::issue_dataset_rdf12(
        &g1.iter()
            .map(|t| {
                Quad::new(
                    t.subject.clone(),
                    t.predicate.clone(),
                    t.object.clone(),
                    GraphName::DefaultGraph,
                )
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let labels: BTreeSet<&str> = ["a", "b", "c"]
        .iter()
        .map(|k| map.get(*k).map(String::as_str).unwrap())
        .collect();
    assert_eq!(
        labels.len(),
        3,
        "issuer map must assign 3 DISTINCT c14n labels to a,b,c: {:?}",
        map
    );
}

/// Vector 5 — self-loop inner-SUBJECT vs inner-OBJECT pair. Both graphs reuse the
/// top-level subject `_:a` inside the triple term, but on opposite sides:
/// G1 `_:a ex:p <<( _:a ex:q _:b )>>` (the loop is on the inner SUBJECT) vs
/// G2 `_:a ex:p <<( _:b ex:q _:a )>>` (the loop is on the inner OBJECT), each
/// with `_:b ex:t "x"`. Brute-force-verified NON-isomorphic. The canon must
/// DIFFER — the inner subject-vs-object asymmetry of the self-loop is preserved.
#[test]
fn vec5_self_loop_inner_subject_vs_object_differs() {
    let g1 = vec![
        Triple::new(
            NamedOrBlankNode::BlankNode(bn("a")),
            iri("http://ex/p"),
            tt(
                NamedOrBlankNode::BlankNode(bn("a")),
                iri("http://ex/q"),
                Term::BlankNode(bn("b")),
            ),
        ),
        Triple::new(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/t"),
            lit("x"),
        ),
    ];
    let g2 = vec![
        Triple::new(
            NamedOrBlankNode::BlankNode(bn("a")),
            iri("http://ex/p"),
            tt(
                NamedOrBlankNode::BlankNode(bn("b")),
                iri("http://ex/q"),
                Term::BlankNode(bn("a")),
            ),
        ),
        Triple::new(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/t"),
            lit("x"),
        ),
    ];
    assert!(
        !is_isomorphic(&g1, &g2),
        "premise: self-loop inner-subject vs inner-object must be non-isomorphic"
    );
    let c1 = sparq_canon::canonicalize_triples_rdf12(&g1).unwrap();
    let c2 = sparq_canon::canonicalize_triples_rdf12(&g2).unwrap();
    assert_ne!(
        c1.lines, c2.lines,
        "self-loop inner-subject vs inner-object must canonicalize differently"
    );
}

/// Vector 6 — boundary: there is NO SHA-384 rdf12 path. The generic STANDARD
/// dataset entry point `canonicalize_quads_with::<Sha384>` fails closed with
/// `CanonError::TripleTerm` on triple-term input (the rdf12 profile is SHA-256
/// only via `canonicalize_rdf12` / `..._with`; the SHA-384 *standard* path never
/// reaches a triple-term canonicalizer). This pins the audit's confirmed
/// boundary: a `*_with::<Sha384>` generic cannot be coaxed into mis-canonicalizing
/// a triple term, because it rejects it before hashing.
#[test]
fn vec6_sha384_standard_path_rejects_triple_terms() {
    let q = Quad::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/asserts"),
        tt(
            NamedOrBlankNode::BlankNode(bn("n")),
            iri("http://ex/p"),
            lit("v"),
        ),
        GraphName::DefaultGraph,
    );
    assert!(
        matches!(
            sparq_canon::canonicalize_quads_with::<Sha384>(std::slice::from_ref(&q)),
            Err(sparq_canon::CanonError::TripleTerm)
        ),
        "standard SHA-384 path must reject triple terms with CanonError::TripleTerm"
    );
    // The rdf12 SHA-384 profile, by contrast, DOES canonicalize the same input
    // (proving the rejection above is the standard path's boundary, not a global
    // SHA-384 limitation).
    assert!(sparq_canon::canonicalize_rdf12_with::<Sha384>(std::slice::from_ref(&q)).is_ok());
}

/// REAL-PATH PROOF that `D` is load-bearing (not a no-op generic): on the
/// symmetric two-bnode cycle, SHA-256 and SHA-384 issue the SAME canonical
/// labels to OPPOSITE input bnodes, so the two relabelling maps differ. If the
/// hash were ever dropped (e.g. a hardcoded `Sha256` left in a call site), this
/// assertion would fail.
#[test]
fn sha384_relabels_differently_from_sha256_on_symmetric_cycle() {
    let ds = symmetric_cycle_dataset();
    let m256 = sparq_canon::issue_dataset_rdf12_with::<Sha256>(&ds).unwrap();
    let m384 = sparq_canon::issue_dataset_rdf12_with::<Sha384>(&ds).unwrap();
    assert_ne!(
        m256, m384,
        "the chosen hash must actually drive the relabelling (D is load-bearing)"
    );
    // Both are still valid bijections onto {c14n0, c14n1}.
    let labels256: std::collections::BTreeSet<_> = m256.values().cloned().collect();
    let labels384: std::collections::BTreeSet<_> = m384.values().cloned().collect();
    let expected: std::collections::BTreeSet<_> = ["c14n0".to_string(), "c14n1".to_string()]
        .into_iter()
        .collect();
    assert_eq!(labels256, expected);
    assert_eq!(labels384, expected);
}

// ---------------------------------------------------------------------------
// 6) Direct tests for `canonicalize_graph_content_rdf12` and the `*_with`
//    generic — the `sparq_core::Graph`-level entry points. [OPUS-4.8] sq-qcnn.14:
//    these were the only public rdf12 functions without any test coverage.
// ---------------------------------------------------------------------------

/// Direct test for `canonicalize_graph_content_rdf12` — the Graph-level entry
/// point for the non-standard triple-term profile. Verifies that a bnode in a
/// loaded Graph is relabelled to `c14n0` and the canonical line is well-formed.
/// Kills "return empty CanonicalGraph" and "skip graph_triples extraction" mutants.
#[test]
fn canonicalize_graph_content_rdf12_relabels_bnode() {
    use sparq_core::Graph;
    let g = Graph::load_str(
        "_:b0 <http://ex/p> <http://ex/o> .\n",
        "ntriples",
    )
    .expect("load_str");
    let c = sparq_canon::canonicalize_graph_content_rdf12(&g).unwrap();
    assert_eq!(c.lines.len(), 1, "one stored triple -> one canonical line");
    assert!(
        c.lines[0].contains("_:c14n0"),
        "bnode must be relabelled to c14n0: {:?}",
        c.lines[0]
    );
    assert!(
        c.lines[0].contains("<http://ex/p>"),
        "predicate must be preserved: {:?}",
        c.lines[0]
    );
    assert_eq!(c.triples.len(), 1, "one line -> one canonical triple");
    // The to_nquads join must produce the canonical doc.
    let doc = c.to_nquads();
    assert_eq!(doc, format!("{}\n", c.lines[0]));
}

/// `canonicalize_graph_content_rdf12` on an EMPTY graph must succeed and return
/// an empty CanonicalGraph (mirrors the standard path's empty-dataset behaviour).
#[test]
fn canonicalize_graph_content_rdf12_empty_graph() {
    use sparq_core::Graph;
    let g = Graph::load_str("", "ntriples").expect("load_str");
    let c = sparq_canon::canonicalize_graph_content_rdf12(&g).unwrap();
    assert!(c.lines.is_empty(), "empty graph -> empty canonical form");
    assert!(c.triples.is_empty());
    assert_eq!(c.to_nquads(), "");
}

/// Direct test for `canonicalize_graph_content_rdf12_with::<D>` (hash-generic).
/// Uses SHA-384. Asserts bnode relabelling and exact line count so mutations to
/// the hash dispatch or the triples extraction are killed.
#[test]
fn canonicalize_graph_content_rdf12_with_sha384_relabels_bnode() {
    use sparq_core::Graph;
    let g = Graph::load_str(
        "_:x <http://ex/q> <http://ex/r> .\n",
        "ntriples",
    )
    .expect("load_str");
    let c = sparq_canon::canonicalize_graph_content_rdf12_with::<Sha384>(&g).unwrap();
    assert_eq!(c.lines.len(), 1, "one stored triple -> one canonical line");
    assert!(
        c.lines[0].contains("_:c14n0"),
        "bnode must be relabelled to c14n0 under SHA-384: {:?}",
        c.lines[0]
    );
    assert!(
        c.lines[0].contains("<http://ex/q>"),
        "predicate must be preserved: {:?}",
        c.lines[0]
    );
    // SHA-256 (default) and SHA-384 agree on a single-bnode, single-triple graph
    // because there is no shared-hash disambiguation (exactly one bnode).
    let c256 = sparq_canon::canonicalize_graph_content_rdf12(&g).unwrap();
    assert_eq!(
        c.lines, c256.lines,
        "single-bnode graph must canonicalize identically under any hash"
    );
}

// ---------------------------------------------------------------------------
// 7) Constrained ground-triple-term variant ([FABLE-5] sq-iaxd): the thin
//    error-on-nested-bnode wrappers. Contract under test:
//    (a) on ground-triple-term input each wrapper is byte-identical to the
//        full profile it delegates to (SHA-256 and SHA-384);
//    (b) any blank node INSIDE a triple term — inner subject, inner object,
//        or deeper — fails closed with `CanonError::NestedBlankNode`;
//    (c) TOP-LEVEL blank nodes stay ordinary RDFC-1.0 bnodes (accepted and
//        relabelled), so the guard's nested/top-level distinction is pinned.
// ---------------------------------------------------------------------------

/// A ground triple term `<<( <s> <p> <o> )>>`.
fn ground_tt() -> Term {
    tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::NamedNode(iri("http://ex/o")),
    )
}

/// A triple term with a blank node in its OBJECT position.
fn tt_inner_obj_bnode(label: &str) -> Term {
    tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::BlankNode(bn(label)),
    )
}

/// A quad asserting `who says <tt>` in the default graph.
fn says_quad(who: &str, object: Term) -> Quad {
    Quad::new(
        NamedOrBlankNode::NamedNode(iri(who)),
        iri("http://ex/says"),
        object,
        GraphName::DefaultGraph,
    )
}

/// Direct test for `canonicalize_rdf12_ground_terms`: a ground-triple-term
/// dataset with a TOP-LEVEL bnode subject is accepted, relabels the top-level
/// bnode, and is byte-identical to the full profile (delegation).
#[test]
fn ground_terms_accepts_and_matches_full_profile() {
    let dataset = vec![
        Quad::new(
            NamedOrBlankNode::BlankNode(bn("holder")),
            iri("http://ex/says"),
            ground_tt(),
            GraphName::DefaultGraph,
        ),
        says_quad("http://ex/issuer", Term::Literal(Literal::new_simple_literal("v"))),
    ];
    let constrained = sparq_canon::canonicalize_rdf12_ground_terms(&dataset).unwrap();
    let full = sparq_canon::canonicalize_rdf12(&dataset).unwrap();
    assert_eq!(
        constrained, full,
        "constrained variant must be byte-identical to the full profile on accepted input"
    );
    assert!(
        constrained.contains("_:c14n0"),
        "top-level bnode must be relabelled: {constrained:?}"
    );
    assert!(
        constrained.contains("<<("),
        "ground triple-term token form must be preserved: {constrained:?}"
    );
}

/// Direct test for `canonicalize_rdf12_ground_terms_with::<Sha384>`: delegation
/// equivalence must hold under a non-default hash profile too.
#[test]
fn ground_terms_with_sha384_matches_full_profile() {
    let dataset = vec![
        Quad::new(
            NamedOrBlankNode::BlankNode(bn("x")),
            iri("http://ex/says"),
            ground_tt(),
            GraphName::DefaultGraph,
        ),
        Quad::new(
            NamedOrBlankNode::BlankNode(bn("y")),
            iri("http://ex/q"),
            Term::BlankNode(bn("x")),
            GraphName::DefaultGraph,
        ),
    ];
    let constrained = sparq_canon::canonicalize_rdf12_ground_terms_with::<Sha384>(&dataset).unwrap();
    let full = sparq_canon::canonicalize_rdf12_with::<Sha384>(&dataset).unwrap();
    assert_eq!(constrained, full, "SHA-384 delegation must be byte-identical");
    // And the SHA-256 default is reachable through the non-generic entry point.
    let c256 = sparq_canon::canonicalize_rdf12_ground_terms(&dataset).unwrap();
    assert_eq!(
        c256,
        sparq_canon::canonicalize_rdf12_ground_terms_with::<Sha256>(&dataset).unwrap(),
        "non-generic entry point must be the SHA-256 profile"
    );
}

/// Direct test for `issue_dataset_rdf12_ground_terms` (+ `_with`): the issuer
/// map on accepted input equals the full profile's, and top-level bnodes are
/// issued canonical labels.
#[test]
fn ground_terms_issuer_map_matches_full_profile() {
    let dataset = vec![
        Quad::new(
            NamedOrBlankNode::BlankNode(bn("only")),
            iri("http://ex/says"),
            ground_tt(),
            GraphName::DefaultGraph,
        ),
    ];
    let m = sparq_canon::issue_dataset_rdf12_ground_terms(&dataset).unwrap();
    assert_eq!(
        m.get("only").map(String::as_str),
        Some("c14n0"),
        "single top-level bnode must issue as c14n0: {m:?}"
    );
    assert_eq!(
        m,
        sparq_canon::issue_dataset_rdf12(&dataset).unwrap(),
        "issuer map must equal the full profile's on accepted input"
    );
    let m384 = sparq_canon::issue_dataset_rdf12_ground_terms_with::<Sha384>(&dataset).unwrap();
    assert_eq!(
        m384,
        sparq_canon::issue_dataset_rdf12_with::<Sha384>(&dataset).unwrap(),
        "SHA-384 issuer map must equal the full profile's"
    );
}

/// Direct test for `canonicalize_triples_rdf12_ground_terms` (+ `_with`): the
/// single-graph wrapper delegates on ground input and keeps the
/// `CanonicalGraph` lines/triples contract.
#[test]
fn ground_terms_triples_matches_full_profile() {
    let t1 = Triple::new(
        NamedOrBlankNode::BlankNode(bn("top")),
        iri("http://ex/says"),
        ground_tt(),
    );
    let t2 = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/q"),
        Term::Literal(Literal::new_simple_literal("leaf")),
    );
    let c = sparq_canon::canonicalize_triples_rdf12_ground_terms(&[t1.clone(), t2.clone()]).unwrap();
    let full = sparq_canon::canonicalize_triples_rdf12(&[t1.clone(), t2.clone()]).unwrap();
    assert_eq!(c.lines, full.lines, "lines must equal the full profile's");
    assert_eq!(c.triples, full.triples, "triples must equal the full profile's");
    assert_eq!(c.lines.len(), 2, "two triples -> two canonical lines");
    let c384 =
        sparq_canon::canonicalize_triples_rdf12_ground_terms_with::<Sha384>(&[t1.clone(), t2.clone()])
            .unwrap();
    let full384 = sparq_canon::canonicalize_triples_rdf12_with::<Sha384>(&[t1, t2]).unwrap();
    assert_eq!(c384.lines, full384.lines, "SHA-384 delegation must agree");
}

/// On triple-term-FREE input the constrained variant agrees byte-for-byte with
/// the STANDARD `rdf-canon`-backed path (inherited from the full profile's
/// standard-path agreement, asserted directly here for the wrapper).
#[test]
fn ground_terms_agrees_with_standard_path_on_triple_term_free_input() {
    let dataset = vec![
        Quad::new(
            NamedOrBlankNode::BlankNode(bn("b0")),
            iri("http://ex/p"),
            Term::BlankNode(bn("b1")),
            GraphName::DefaultGraph,
        ),
        Quad::new(
            NamedOrBlankNode::BlankNode(bn("b1")),
            iri("http://ex/q"),
            Term::Literal(Literal::new_simple_literal("v")),
            GraphName::DefaultGraph,
        ),
    ];
    assert_eq!(
        sparq_canon::canonicalize_rdf12_ground_terms(&dataset).unwrap(),
        sparq_canon::canonicalize(&dataset).unwrap(),
        "constrained variant must equal the standard path on RDF-1.1 input"
    );
}

/// (b) Error contract: a bnode in the triple term's OBJECT position fails
/// closed with `NestedBlankNode` on every wrapper — canonicalize, issuer map,
/// and the `_with` siblings.
#[test]
fn nested_bnode_in_object_position_fails_closed() {
    let dataset = vec![says_quad("http://ex/a", tt_inner_obj_bnode("inner"))];
    assert!(matches!(
        sparq_canon::canonicalize_rdf12_ground_terms(&dataset),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
    assert!(matches!(
        sparq_canon::canonicalize_rdf12_ground_terms_with::<Sha384>(&dataset),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
    assert!(matches!(
        sparq_canon::issue_dataset_rdf12_ground_terms(&dataset),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
    assert!(matches!(
        sparq_canon::issue_dataset_rdf12_ground_terms_with::<Sha384>(&dataset),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
    // Contrast (non-vacuity): the FULL profile canonicalizes the same input.
    assert!(
        sparq_canon::canonicalize_rdf12(&dataset).is_ok(),
        "the full descent must still accept what the constrained variant rejects"
    );
}

/// (b) Error contract: a bnode in the triple term's SUBJECT position (a
/// `NamedOrBlankNode`, so it can be blank) also fails closed.
#[test]
fn nested_bnode_in_subject_position_fails_closed() {
    let inner_subj = tt(
        NamedOrBlankNode::BlankNode(bn("inner")),
        iri("http://ex/p"),
        Term::Literal(Literal::new_simple_literal("v")),
    );
    let triples = vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        inner_subj,
    )];
    assert!(matches!(
        sparq_canon::canonicalize_triples_rdf12_ground_terms(&triples),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
    assert!(matches!(
        sparq_canon::canonicalize_triples_rdf12_ground_terms_with::<Sha384>(&triples),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
}

/// (b) Error contract: a bnode at nesting depth 2 (a triple term inside a
/// triple term) is found by the recursive guard.
#[test]
fn nested_bnode_at_depth_two_fails_closed() {
    let deepest = tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/x")),
        iri("http://ex/p"),
        Term::BlankNode(bn("deep")),
    );
    let mid = tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/y")),
        iri("http://ex/q"),
        deepest,
    );
    let dataset = vec![says_quad("http://ex/a", mid)];
    assert!(matches!(
        sparq_canon::canonicalize_rdf12_ground_terms(&dataset),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
}

/// (c) The guard must NOT be tripped by top-level bnodes anywhere a bnode can
/// legally sit in RDF-1.1 (subject, object, graph name) — only by bnodes
/// inside triple terms. A dataset mixing all three top-level positions with a
/// ground triple term is accepted and isomorphism-stable (relabel + reorder).
#[test]
fn top_level_bnodes_everywhere_are_accepted_and_isomorphism_stable() {
    let make = |s: &str, o: &str, g: &str, swap: bool| -> String {
        let q1 = Quad::new(
            NamedOrBlankNode::BlankNode(bn(s)),
            iri("http://ex/says"),
            ground_tt(),
            GraphName::BlankNode(bn(g)),
        );
        let q2 = Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/a")),
            iri("http://ex/knows"),
            Term::BlankNode(bn(o)),
            GraphName::DefaultGraph,
        );
        let dataset = if swap { vec![q2, q1] } else { vec![q1, q2] };
        sparq_canon::canonicalize_rdf12_ground_terms(&dataset).unwrap()
    };
    let a = make("s1", "o1", "g1", false);
    let b = make("other1", "other2", "other3", true);
    assert_eq!(
        a, b,
        "isomorphic ground-TT datasets must canonicalize identically"
    );
    assert!(a.contains("<<("), "triple-term token preserved: {a:?}");
}

/// The new error variant's `Display` must self-describe (used in fail-closed
/// logs); check the message names the nested-bnode condition and the full
/// profile's escape hatch.
#[test]
fn nested_blank_node_error_display() {
    let msg = sparq_canon::CanonError::NestedBlankNode.to_string();
    assert!(
        msg.contains("nested") && msg.contains("triple term"),
        "Display must name the condition: {msg:?}"
    );
    assert!(
        msg.contains("canonicalize_rdf12"),
        "Display must point at the full-descent escape hatch: {msg:?}"
    );
}

// ---------------------------------------------------------------------------
// 8) Graph-level constrained wrappers ([FABLE-5] sq-l3pk7):
//    `canonicalize_graph_content_rdf12_ground_terms` (+ `_with`) — the
//    `sparq_core::Graph` parity siblings of the sq-iaxd wrapper family.
//    Contract: (a) equivalent to composing `graph_triples` with
//    `canonicalize_triples_rdf12_ground_terms` and byte-identical to the full
//    graph_content profile on ground input; (b) fail closed with
//    `NestedBlankNode` on any bnode inside a stored triple term (which the
//    FULL graph_content profile accepts — kills a "delegate without guard"
//    mutant); (c) the `_with` hash generic is honoured.
// ---------------------------------------------------------------------------

/// Direct test for `canonicalize_graph_content_rdf12_ground_terms`: a stored
/// ground triple term plus a top-level bnode is accepted, byte-identical to
/// the full profile, and identical to the documented composition it replaces.
#[test]
fn graph_content_ground_terms_matches_full_profile_and_composition() {
    use sparq_core::Graph;
    let g = Graph::load_str(
        "<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> <http://ex/o> )>> .\n\
         _:top <http://ex/knows> <http://ex/a> .\n",
        "ntriples",
    )
    .expect("load_str");
    let c = sparq_canon::canonicalize_graph_content_rdf12_ground_terms(&g).unwrap();
    // Non-vacuity: the triple term survives and the top-level bnode relabels.
    assert_eq!(c.lines.len(), 2, "two stored triples -> two canonical lines");
    assert!(
        c.lines.iter().any(|l| l.contains("<<(")),
        "triple-term token preserved: {:?}",
        c.lines
    );
    assert!(
        c.lines.iter().any(|l| l.contains("_:c14n0")),
        "top-level bnode must relabel to c14n0: {:?}",
        c.lines
    );
    // (a) Byte-identical to the FULL graph_content profile on ground input
    // (thin guard + delegate) …
    let full = sparq_canon::canonicalize_graph_content_rdf12(&g).unwrap();
    assert_eq!(c.lines, full.lines);
    assert_eq!(c.triples, full.triples);
    // … and to the composition the wrapper is a parity sibling of.
    let triples = sparq_canon::graph_triples(&g).unwrap();
    let composed = sparq_canon::canonicalize_triples_rdf12_ground_terms(&triples).unwrap();
    assert_eq!(c.lines, composed.lines);
    assert_eq!(c.triples, composed.triples);
}

/// (b) Error contract for the Graph-level wrapper: a blank node INSIDE a
/// stored triple term fails closed with `NestedBlankNode`, while the FULL
/// graph_content profile still accepts the same graph. Kills a mutant that
/// drops the guard and delegates straight to `canonicalize_graph_content_rdf12`.
#[test]
fn graph_content_ground_terms_rejects_nested_bnode() {
    use sparq_core::Graph;
    let g = Graph::load_str(
        "<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> _:inner )>> .\n",
        "ntriples",
    )
    .expect("load_str");
    assert!(matches!(
        sparq_canon::canonicalize_graph_content_rdf12_ground_terms(&g),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
    assert!(matches!(
        sparq_canon::canonicalize_graph_content_rdf12_ground_terms_with::<Sha384>(&g),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
    // Contrast (non-vacuity): the full descent accepts what the constrained
    // Graph-level variant rejects.
    assert!(
        sparq_canon::canonicalize_graph_content_rdf12(&g).is_ok(),
        "the full graph_content descent must still accept nested bnodes"
    );
}

/// Direct test for `canonicalize_graph_content_rdf12_ground_terms_with::<D>`:
/// the hash generic delegates faithfully — SHA-384 output equals the full
/// profile's SHA-384 output on a ground graph, and the default entry point is
/// exactly the `Sha256` instantiation.
#[test]
fn graph_content_ground_terms_with_sha384_matches_full_profile() {
    use sparq_core::Graph;
    let g = Graph::load_str(
        "_:x <http://ex/says> <<( <http://ex/s> <http://ex/p> \"v\" )>> .\n",
        "ntriples",
    )
    .expect("load_str");
    let c384 =
        sparq_canon::canonicalize_graph_content_rdf12_ground_terms_with::<Sha384>(&g).unwrap();
    let full384 = sparq_canon::canonicalize_graph_content_rdf12_with::<Sha384>(&g).unwrap();
    assert_eq!(c384.lines, full384.lines);
    assert!(
        c384.lines[0].contains("_:c14n0"),
        "top-level bnode must relabel under SHA-384: {:?}",
        c384.lines
    );
    // Default = Sha256 instantiation, exactly.
    let c = sparq_canon::canonicalize_graph_content_rdf12_ground_terms(&g).unwrap();
    let c256 =
        sparq_canon::canonicalize_graph_content_rdf12_ground_terms_with::<Sha256>(&g).unwrap();
    assert_eq!(c.lines, c256.lines);
    assert_eq!(c.triples, c256.triples);
}

// ---------------------------------------------------------------------------
// [FABLE-5] sq-x3oj2: crate-wide triple-term nesting-depth bound.
// Every profile entry point must fail closed with `TripleTermDepthExceeded`
// on a chain one past `MAX_TRIPLE_TERM_DEPTH`, and still accept a chain
// exactly AT the bound (so the guard is a bound, not a blanket reject).
// Knocking out the iterative pre-check turns each Err assert into Ok — red.
// ---------------------------------------------------------------------------

/// Ground triple-term chain of exactly `depth` nesting levels, built with a
/// loop (the test helper must not itself recurse).
fn ground_chain(depth: usize) -> Term {
    let mut obj = Term::NamedNode(iri("http://ex/leaf"));
    for _ in 0..depth {
        obj = tt(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            obj,
        );
    }
    obj
}

fn quad_with_object(o: Term) -> Quad {
    Quad::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        o,
        GraphName::DefaultGraph,
    )
}

/// Full-profile dataset entry points: at the bound canonicalizes (and the
/// issuer map is empty — the chain is ground); one past it fails closed.
#[test]
fn depth_bound_full_profile_dataset() {
    let at = [quad_with_object(ground_chain(sparq_canon::MAX_TRIPLE_TERM_DEPTH))];
    let over = [quad_with_object(ground_chain(
        sparq_canon::MAX_TRIPLE_TERM_DEPTH + 1,
    ))];
    let canon = sparq_canon::canonicalize_rdf12(&at).expect("depth == MAX must canonicalize");
    assert!(canon.contains("<<("), "triple-term token preserved: truncated");
    assert!(sparq_canon::issue_dataset_rdf12(&at)
        .expect("depth == MAX must issue")
        .is_empty());
    assert!(matches!(
        sparq_canon::canonicalize_rdf12(&over),
        Err(sparq_canon::CanonError::TripleTermDepthExceeded)
    ));
    assert!(matches!(
        sparq_canon::issue_dataset_rdf12(&over),
        Err(sparq_canon::CanonError::TripleTermDepthExceeded)
    ));
}

/// Single-graph (triples) entry points, full + constrained.
#[test]
fn depth_bound_triples_entry_points() {
    let at = [Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        ground_chain(sparq_canon::MAX_TRIPLE_TERM_DEPTH),
    )];
    let over = [Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        ground_chain(sparq_canon::MAX_TRIPLE_TERM_DEPTH + 1),
    )];
    assert_eq!(
        sparq_canon::canonicalize_triples_rdf12(&at)
            .expect("depth == MAX must canonicalize")
            .lines
            .len(),
        1
    );
    assert!(matches!(
        sparq_canon::canonicalize_triples_rdf12(&over),
        Err(sparq_canon::CanonError::TripleTermDepthExceeded)
    ));
    assert!(matches!(
        sparq_canon::canonicalize_triples_rdf12_ground_terms(&over),
        Err(sparq_canon::CanonError::TripleTermDepthExceeded)
    ));
}

/// Constrained (ground-terms) dataset entry points: the depth guard runs
/// BEFORE the nested-bnode guard, so an over-deep chain that ALSO carries a
/// nested blank node reports `TripleTermDepthExceeded`, not `NestedBlankNode`
/// (the ground guard would otherwise walk the over-deep chain to find it).
#[test]
fn depth_bound_precedes_ground_guard() {
    let mut obj = Term::BlankNode(bn("innermost"));
    for _ in 0..(sparq_canon::MAX_TRIPLE_TERM_DEPTH + 1) {
        obj = tt(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            obj,
        );
    }
    let over = [quad_with_object(obj)];
    assert!(matches!(
        sparq_canon::canonicalize_rdf12_ground_terms(&over),
        Err(sparq_canon::CanonError::TripleTermDepthExceeded)
    ));
    assert!(matches!(
        sparq_canon::issue_dataset_rdf12_ground_terms(&over),
        Err(sparq_canon::CanonError::TripleTermDepthExceeded)
    ));
    // At the bound, the SAME shape (nested bnode, depth == MAX) is the ground
    // guard's territory again: `NestedBlankNode`, proving the precedence
    // flip happens exactly at the bound.
    let mut obj = Term::BlankNode(bn("innermost"));
    for _ in 0..sparq_canon::MAX_TRIPLE_TERM_DEPTH {
        obj = tt(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            obj,
        );
    }
    assert!(matches!(
        sparq_canon::canonicalize_rdf12_ground_terms(&[quad_with_object(obj)]),
        Err(sparq_canon::CanonError::NestedBlankNode)
    ));
}

// ---------------------------------------------------------------------------
// Poison-graph guard (#5469 / #5358): the HNDQ permutation enumeration must be
// lazy and budgeted, so the guard fails closed with `CanonError::Canonicalization`
// instead of materializing k! permutations and aborting on allocation failure.
// ---------------------------------------------------------------------------

/// An `n`-node blank-node clique (every node linked to every node, self-loops
/// included): every node shares one first-degree hash, and every HNDQ call sees
/// one related-hash group of size `n - 1`.
fn bnode_clique(n: usize) -> Vec<Quad> {
    let p = iri("http://example.com/p");
    let mut quads = Vec::with_capacity(n * n);
    for i in 0..n {
        for j in 0..n {
            quads.push(Quad::new(
                NamedOrBlankNode::BlankNode(bn(&format!("e{i}"))),
                p.clone(),
                Term::BlankNode(bn(&format!("e{j}"))),
                GraphName::DefaultGraph,
            ));
        }
    }
    quads
}

fn assert_fails_closed(quads: &[Quad], what: &str) {
    let start = std::time::Instant::now();
    let res = sparq_canon::canonicalize_rdf12(quads);
    assert!(
        matches!(res, Err(sparq_canon::CanonError::Canonicalization(_))),
        "{what}: expected a fail-closed Canonicalization error, got {res:?}"
    );
    // Generous wall-clock ceiling: the guard must trip in bounded work, not after
    // walking a factorial-sized permutation space.
    assert!(
        start.elapsed() < std::time::Duration::from_secs(60),
        "{what}: guard took {:?} to trip",
        start.elapsed()
    );
}

/// The W3C suite's own negative vector (10-node clique) — the one entry the
/// agreement test above skips because the standard path rejects it.
#[test]
fn rdf12_fails_closed_on_suite_poison_graph_test074() {
    let quads = load_quads(&testdata().join("rdfc10/test074-in.nq"));
    assert_fails_closed(&quads, "test074");
}

/// A 13-node clique: 12! (~4.8e8) permutations per HNDQ call. Eager
/// materialization needs tens of GiB before the guard is reachable.
#[test]
fn rdf12_fails_closed_on_13_node_clique() {
    assert_fails_closed(&bnode_clique(13), "13-node clique");
}
