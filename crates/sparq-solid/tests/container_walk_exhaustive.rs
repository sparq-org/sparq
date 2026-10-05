//! [FABLE-5] sq-sqtk2.1 (epic sq-sqtk2; research/mechanized-proof-program.md §3.1 property
//! A-2, §5 bead 1) — EXHAUSTIVELY TESTED (bounded domain): container-default ACL
//! discovery. Kani string costs make a symbolic-IRI proof of the nearest-ancestor
//! SELECTION intractable (the walk's TERMINATION lemma is Kani-proved in
//! `src/decide.rs::kani_proofs::parent_iri_strictly_shortens`), so this test enumerates a
//! finite resource/control-doc domain COMPLETELY and checks `PodStore::resolve_acl`
//! against an independent reference on every single case.
//!
//! THE DOMAIN (complete, and pinned by an explicit case count so a generator bug cannot
//! silently shrink it): every container and every document under `https://pod.ex/` whose
//! path has ≤ 3 segments drawn from the alphabet {`a`, `b`}, crossed with EVERY assignment
//! of {no control doc, `.acl`, `.acr`} to the resource itself and to each of its ancestor
//! containers — 1551 datasets in all. Anything outside this domain (longer paths, other
//! segment names, query strings, non-slash hierarchies) is NOT covered here; the WAC/ACP
//! conformance corpora + the differential oracle remain the tier of record for full-spec
//! semantics.
//!
//! THE REFERENCE is computed from the generator's own segment structure (the ancestor
//! chain is built from the segment vector the resource was constructed from), NOT by
//! re-running any string-walk logic from the crate — so agreement is meaningful:
//!   • a control doc on the resource itself governs with `AccessTo` scope;
//!   • else the NEAREST ancestor container holding a control doc governs with `Default`;
//!   • else `resolve_acl` is `None` (the caller fails closed).

use sparq_core::Graph;
use sparq_solid::{AclScope, PodStore};

const HOST: &str = "https://pod.ex/";
const SEGMENTS: [&str; 2] = ["a", "b"];
const MAX_DEPTH: usize = 3;

/// The control-doc choice at one position of the chain.
#[derive(Clone, Copy, PartialEq)]
enum Doc {
    Absent,
    Acl,
    Acr,
}
const DOC_CHOICES: [Doc; 3] = [Doc::Absent, Doc::Acl, Doc::Acr];

impl Doc {
    fn suffix(self) -> Option<&'static str> {
        match self {
            Doc::Absent => None,
            Doc::Acl => Some(".acl"),
            Doc::Acr => Some(".acr"),
        }
    }
}

/// The container IRI for a segment prefix: `https://pod.ex/` + `s1/` + … + `sk/`.
fn container_iri(segs: &[&str]) -> String {
    let mut iri = HOST.to_owned();
    for s in segs {
        iri.push_str(s);
        iri.push('/');
    }
    iri
}

/// `resource` plus its ancestor containers, NEAREST-FIRST, ending at the pod root —
/// derived from the generator's segment vector, not from IRI string surgery.
fn chain(segs: &[&str], is_container: bool) -> Vec<String> {
    let mut chain = Vec::new();
    if is_container {
        chain.push(container_iri(segs));
    } else {
        // A document lives IN its parent container: HOST + s1/ + … + s(d-1)/ + sd.
        let mut iri = container_iri(&segs[..segs.len() - 1]);
        iri.push_str(segs[segs.len() - 1]);
        chain.push(iri);
    }
    for k in (0..segs.len()).rev() {
        chain.push(container_iri(&segs[..k]));
    }
    chain
}

/// One dataset: a named graph (with a dummy triple) per present control doc.
fn dataset(docs: &[(String, Doc)]) -> Graph {
    let mut nquads = String::new();
    for (owner, doc) in docs {
        if let Some(suffix) = doc.suffix() {
            let control = format!("{}{}", owner, suffix);
            nquads.push_str(&format!(
                "<{c}#auth> <https://ex.dev/ns#p> \"1\" <{c}> .\n",
                c = control
            ));
        }
    }
    Graph::load_dataset(&nquads, "nquads").expect("dataset loads")
}

/// Every assignment of `DOC_CHOICES` to `n` positions (3^n assignments).
fn assignments(n: usize) -> Vec<Vec<Doc>> {
    let mut out = vec![Vec::new()];
    for _ in 0..n {
        let mut next = Vec::with_capacity(out.len() * DOC_CHOICES.len());
        for prefix in &out {
            for &d in &DOC_CHOICES {
                let mut a = prefix.clone();
                a.push(d);
                next.push(a);
            }
        }
        out = next;
    }
    out
}

/// All segment vectors of exactly `depth` segments over the alphabet.
fn segment_vectors(depth: usize) -> Vec<Vec<&'static str>> {
    let mut out = vec![Vec::new()];
    for _ in 0..depth {
        let mut next = Vec::with_capacity(out.len() * SEGMENTS.len());
        for prefix in &out {
            for &s in &SEGMENTS {
                let mut v = prefix.clone();
                v.push(s);
                next.push(v);
            }
        }
        out = next;
    }
    out
}

#[test]
fn resolve_acl_matches_nearest_ancestor_reference_exhaustively() {
    let mut cases = 0usize;
    for depth in 0..=MAX_DEPTH {
        for segs in segment_vectors(depth) {
            // depth 0 has only the root container; depth ≥ 1 has a container AND a
            // document form (distinct resources with distinct own-ACL names).
            let forms: &[bool] = if depth == 0 { &[true] } else { &[true, false] };
            for &is_container in forms {
                let walk = chain(&segs, is_container);
                for assignment in assignments(walk.len()) {
                    cases += 1;
                    let docs: Vec<(String, Doc)> = walk
                        .iter()
                        .cloned()
                        .zip(assignment.iter().copied())
                        .collect();
                    let store = PodStore::new(dataset(&docs));
                    let got = store.resolve_acl(&walk[0]);

                    // The independent reference: first chain position with a control
                    // doc governs — position 0 by accessTo, any later one by default.
                    let expected = docs
                        .iter()
                        .enumerate()
                        .find_map(|(k, (owner, doc))| {
                            doc.suffix().map(|suffix| {
                                (
                                    format!("{}{}", owner, suffix),
                                    if k == 0 { AclScope::AccessTo } else { AclScope::Default },
                                )
                            })
                        });

                    match (got, expected) {
                        (None, None) => {}
                        (Some(eff), Some((acl, scope))) => {
                            assert_eq!(
                                eff.acl.as_str(),
                                acl,
                                "wrong governing ACL for <{}>",
                                walk[0]
                            );
                            assert_eq!(eff.scope, scope, "wrong scope for <{}>", walk[0]);
                        }
                        (got, expected) => panic!(
                            "resolve_acl(<{}>) diverged: got {:?}, expected {:?}",
                            walk[0], got, expected
                        ),
                    }
                }
            }
        }
    }
    // The domain really was enumerated completely: Σ_d resources(d) × 3^(d+1) =
    // 1×3 + 4×9 + 8×27 + 16×81 = 1551. A generator regression fails loudly here.
    assert_eq!(cases, 1551, "the bounded domain must be enumerated completely");
}

/// Deterministic tie pin (an IMPLEMENTATION pin, not a spec claim): when a resource has
/// BOTH `<R>.acl` and `<R>.acr`, discovery prefers `.acl` (the fixed suffix probe order).
/// WAC and ACP each see only their own suffix in practice; this pins that the mixed case
/// stays deterministic rather than asserting Solid-spec semantics for it.
#[test]
fn own_acl_beats_own_acr_deterministically() {
    let nquads = "\
<https://pod.ex/d.acl#a> <https://ex.dev/ns#p> \"1\" <https://pod.ex/d.acl> .\n\
<https://pod.ex/d.acr#a> <https://ex.dev/ns#p> \"1\" <https://pod.ex/d.acr> .\n";
    let store = PodStore::new(Graph::load_dataset(nquads, "nquads").expect("loads"));
    let eff = store.resolve_acl("https://pod.ex/d").expect("own control doc");
    assert_eq!(eff.acl.as_str(), "https://pod.ex/d.acl");
    assert_eq!(eff.scope, AclScope::AccessTo);
}
