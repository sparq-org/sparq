//! GOLDEN end-to-end test for the N3 proof-admissibility reduction — the
//! `research/security-properties-ontology-design.md` **§4.3.3 worked example**,
//! run through the public [`sparq_trust::admissible`] API over `sparq-reason`'s
//! N3 engine.
//!
//! The load-bearing invariant (design §4.3.3): the admissible set for the
//! current sparq estate is **EMPTY under Alice's strict preference** (the honest
//! "no admissible proof" refusal) and **NON-EMPTY under the relaxed one** (so the
//! gate is demonstrably not vacuously empty).
//!
//! 🤖 SPARQ agent — sq-ufsi9 (epic sq-0dksu), Phase 2. Runs only with
//! the `secprop-admissibility` feature; the `feature-matrix.yml` leg EXECUTES it.
#![cfg(feature = "secprop-admissibility")]

use sparq_trust::admissible;

const M: &str = "https://sparq.dev/ns/zk#poseidon2-rdfc10-v1"; // the string-canonical method

/// §4.3.3 / §5a — the `string-canonical` (`zk:poseidon2-rdfc10-v1`) annotation
/// block: the per-dimension levels the method is recorded at. assurance is at
/// most `Claimed` (the default; `Proven` is barred while `sq-qhy4` is open).
/// The `secx:assurance`/`auditStatus` are carried on each assertion node but do
/// NOT participate in the order discharge (the assurance gate is its OWN
/// dimension — see the dedicated assurance assertion below).
const STRING_CANONICAL_ANNOTATIONS: &str = "\
zk:poseidon2-rdfc10-v1\n\
  secx:hasProperty [ secx:property secx:ZeroKnowledgeType ; secx:level secx:ComputationalZK ] ,\n\
                   [ secx:property secx:Soundness ; secx:level secx:KnowledgeSound ] ,\n\
                   [ secx:property secx:PostQuantumForgery ; secx:level secx:PQForgeable ] ,\n\
                   [ secx:property secx:SelectiveDisclosure ; secx:level secx:SelectivelyDisclosable ] ,\n\
                   [ secx:property secx:SingleUse ; secx:level secx:Replayable ] ,\n\
                   [ secx:property secx:UnlinkabilityScope ; secx:level secx:PerPresentation ] ,\n\
                   [ secx:property secx:AssuranceLevel ; secx:level secx:Claimed ] .\n";

/// §4.3.3 — **Alice's STRICT preference**: cross-presentation unlinkable, PQ-
/// forgery-resistant, audited (Proven). The PQ `eq PQForgeryResistant` is modelled
/// as `gteq PQForgeryResistant` (the top of the PostQuantumForgery order, so
/// `gteq`-top ≡ `eq`-top — exact for a top-level requirement).
const ALICE_STRICT_POLICY: &str = "\
zk:cUnlink odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:CrossPresentation .\n\
zk:cPQ     odrl:leftOperand secx:requiresPostQuantumForgery ; odrl:operator odrl:gteq ; odrl:rightOperand secx:PQForgeryResistant .\n\
zk:cAssur  odrl:leftOperand secx:requiresAssurance          ; odrl:operator odrl:gteq ; odrl:rightOperand secx:Proven .\n";

const ALICE_STRICT_CONSTRAINTS: &[&str] = &[
    "https://sparq.dev/ns/zk#cUnlink",
    "https://sparq.dev/ns/zk#cPQ",
    "https://sparq.dev/ns/zk#cAssur",
];

/// §4.3.3 — the **RELAXED preference**: assurance ≥ Claimed, PQ dropped,
/// unlinkability ≥ PerPresentation — under which `string-canonical` IS admissible.
const ALICE_RELAXED_POLICY: &str = "\
zk:rUnlink odrl:leftOperand secx:requiresUnlinkabilityScope ; odrl:operator odrl:gteq ; odrl:rightOperand secx:PerPresentation .\n\
zk:rAssur  odrl:leftOperand secx:requiresAssurance          ; odrl:operator odrl:gteq ; odrl:rightOperand secx:Claimed .\n";

const ALICE_RELAXED_CONSTRAINTS: &[&str] =
    &["https://sparq.dev/ns/zk#rUnlink", "https://sparq.dev/ns/zk#rAssur"];

/// THE GOLDEN INVARIANT: empty admissible set under the strict preference.
#[test]
fn string_canonical_is_inadmissible_under_alices_strict_preference() {
    let a = admissible(
        M,
        ALICE_STRICT_CONSTRAINTS,
        ALICE_STRICT_POLICY,
        STRING_CANONICAL_ANNOTATIONS,
    )
    .expect("reasoning failed");

    assert!(
        !a.admissible,
        "string-canonical must be INADMISSIBLE under Alice's strict preference (design §4.3.3): \
         got admissible, unsatisfied={:?}",
        a.unsatisfied
    );
    // It fails on ALL THREE: unlinkability (Per < Cross), PQ (PQForgeable < resistant),
    // and assurance (Claimed < Proven) — the honest, fully-explained refusal.
    let mut un = a.unsatisfied.clone();
    un.sort();
    let mut want = ALICE_STRICT_CONSTRAINTS.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    want.sort();
    assert_eq!(
        un, want,
        "every strict constraint must be unsatisfied (it fails on unlinkability, PQ, AND assurance)"
    );
}

/// THE GOLDEN INVARIANT (complement): non-empty under the relaxed preference —
/// the gate is demonstrably NOT vacuously empty.
#[test]
fn string_canonical_is_admissible_under_the_relaxed_preference() {
    let a = admissible(
        M,
        ALICE_RELAXED_CONSTRAINTS,
        ALICE_RELAXED_POLICY,
        STRING_CANONICAL_ANNOTATIONS,
    )
    .expect("reasoning failed");

    assert!(
        a.admissible,
        "string-canonical MUST be admissible under the relaxed preference (design §4.3.3 — \
         proving the gate is not vacuously empty): unsatisfied={:?}",
        a.unsatisfied
    );
    assert!(a.unsatisfied.is_empty(), "no constraint may be unsatisfied when admissible");
}

/// The assurance gate is "just another dimension with the same discharge rule"
/// (design §4.3.2): `requiresAssurance gteq Proven` mechanically removes the
/// (Claimed) method — this is how the open external audit (`sq-qhy4`) enters the
/// DATA FLOW, not merely the prose.
#[test]
fn assurance_gate_removes_every_unaudited_method() {
    // ONLY the assurance constraint: Claimed < Proven ⇒ inadmissible.
    let a = admissible(
        M,
        &["https://sparq.dev/ns/zk#cAssur"],
        ALICE_STRICT_POLICY,
        STRING_CANONICAL_ANNOTATIONS,
    )
    .expect("reasoning failed");
    assert!(!a.admissible, "requiresAssurance gteq Proven must remove a Claimed-only method");

    // Drop the bar to Claimed and the SAME method clears the assurance gate.
    let relaxed_assur =
        "zk:cA odrl:leftOperand secx:requiresAssurance ; odrl:operator odrl:gteq ; odrl:rightOperand secx:Claimed .";
    let a2 = admissible(
        M,
        &["https://sparq.dev/ns/zk#cA"],
        relaxed_assur,
        STRING_CANONICAL_ANNOTATIONS,
    )
    .expect("reasoning failed");
    assert!(a2.admissible, "assurance gteq Claimed is met by a Claimed annotation");
}
