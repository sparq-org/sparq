//! sq-39kps — **POST-`Slug` `.acl` escalation corpus** (dotted / encoded /
//! percent / unicode / trailing-dot variants) as DATA plus a DECISION-layer assertion
//! harness.
//!
//! # What this is (and is NOT)
//!
//! A self-contained adversarial corpus of the POST child-name variants that must never
//! let an [`acl:Append`](https://www.w3.org/ns/auth/acl#Append)-only requester mint or
//! shadow a control document (`<child>.acl`) without `acl:Control`. It is **DATA +
//! ASSERTION**, not server/HTTP wiring: the actual POST chokepoint guard is bead
//! `sq-gg0qq.5`'s job. This module gives that bead its adversarial *acceptance bar* — a
//! corpus of the exact evasions (percent-encoded `d0.ttl%2Eacl`, unicode full-width
//! `d0.ttl．acl`, trailing-dot `d0.ttl.acl.`, the container's own `%2Eacl`) a naive
//! exact-suffix guard misses — so the guard cannot be armed vacuously (CTH + roborev
//! MISSED this bug originally).
//!
//! # The invariant (fail-closed / answer-safety)
//!
//! For every corpus entry, once the raw child name is decoded/normalised to its
//! *effective* form it names the `.acl` control document of a governed resource
//! ([`is_acl_shaped`] ⇒ `Control` is the required mode), and a requester WITHOUT `Control`
//! over that governed resource gets a **fail-closed** decision — [`PodStore::decide`]`(…,
//! governed_resource, Mode::Control).allow == false` — via `sparq-solid`'s decision
//! surface. No `.acl`-shaped child name lets a non-`Control` requester obtain a decision
//! that creates or shadows a
//! control document.
//!
//! # Anti-vacuity (the sq-og8u8 domain-coverage discipline)
//!
//! The corpus ships a **cover witness**: at least one entry is a *genuine* escalation
//! attempt — its raw form evades a naive exact-case `ends_with(".acl")` guard yet decodes
//! to an `.acl` document — and a `#[cfg(kani)]` proof pins that this interesting input is
//! present, adversarial, and reachable under the harness bounds (per
//! `research/mechanized-proof-program.md` §5.1–5.2), so the suite cannot silently
//! assume-prune the escalations it exists to test.

use crate::{Mode, PodStore, Session, ALICE, BOB, CAROL, WAC_NQUADS};
use sparq_core::Graph;

/// The parent container the corpus posts into. In [`WAC_NQUADS`] `priv0/` has NO own
/// `.acl`, so it (and the resources under it) inherit the root `.acl`'s `acl:default`,
/// which grants `Control` to **alice only** — every other principal (bob, carol,
/// anonymous) holds NO mode there. That makes `priv0/` the natural escalation target.
pub const PARENT: &str = "https://pod.ex/priv0/";

/// One POST child-name escalation vector.
///
/// The attacker POSTs `raw_child` (a `Slug` value / literal path segment) into
/// [`PARENT`]. It decodes/normalises to `effective_child`, which names the
/// control document `<governed_stem>.acl` — i.e. the ACL that governs the resource at
/// `governed_stem` (relative to [`PARENT`]; the EMPTY stem is the container's OWN `.acl`,
/// which governs the container itself). Authoring that ACL is an `acl:Control` operation
/// **on the governed resource**, so the decision-layer test is `decide(session,
/// governed_resource, Control)`: a requester WITHOUT `Control` over the governed resource
/// must be refused (they cannot create or shadow its control document), while the owner
/// (alice) — who legitimately holds `Control` — is allowed, keeping the denial NON-vacuous.
#[derive(Debug, Clone, Copy)]
pub struct EscalationVector {
    /// Short unique label, surfaced in [`EscalationFailure`] reports.
    pub name: &'static str,
    /// The RAW child name the attacker submits (pre-decode / pre-normalisation).
    pub raw_child: &'static str,
    /// The EFFECTIVE child name after percent-decoding, compatibility dot-mapping, and
    /// trailing dot/space canonicalisation — the stored name, always `<governed_stem>.acl`.
    pub effective_child: &'static str,
    /// The resource the minted `.acl` would GOVERN, relative to [`PARENT`]. `""` = the
    /// container's own `.acl` (governs [`PARENT`] itself); otherwise a resource path
    /// (e.g. `"d0.ttl"` ⇒ the `.acl` shadows `PARENT + "d0.ttl"`). MUST name a
    /// materialized resource so the owner-allowed non-vacuity anchor holds.
    pub governed_stem: &'static str,
    /// `true` iff the RAW form evades a naive exact-case `ends_with(".acl")` guard (i.e.
    /// a vulnerable server that checks the un-decoded slug would MINT it) — the marker of
    /// a *genuine* escalation attempt. `false` for the plainly-`.acl` baseline variants.
    pub raw_evades_naive_check: bool,
    /// The spec-declared expected verdict for a non-`Control` requester: ALWAYS `false`
    /// (fail-closed). Flipping this to `true` must make the harness report a failure —
    /// the bead's non-vacuity spot-check.
    pub expect_allow: bool,
}

const fn ev(
    name: &'static str,
    raw_child: &'static str,
    effective_child: &'static str,
    governed_stem: &'static str,
    raw_evades_naive_check: bool,
) -> EscalationVector {
    EscalationVector {
        name,
        raw_child,
        effective_child,
        governed_stem,
        raw_evades_naive_check,
        expect_allow: false,
    }
}

/// The escalation corpus. Every entry decodes to `<governed_stem>.acl` — the control
/// document for a MATERIALIZED resource (the container `priv0/` for the empty stem, the
/// existing document `d0.ttl` otherwise) — so the owner-allowed anchor is real and the
/// non-owner denial is non-vacuous. The `raw_evades_naive_check = true` entries are the
/// genuine escalations a naive exact-suffix guard misses.
pub static CORPUS: &[EscalationVector] = &[
    // ── OWN-CONTAINER `.acl` (governs `priv0/` itself — the highest-impact shadow) ────
    // baseline the naive guard catches, then the percent + unicode evasions it misses.
    ev("own-acl-plain", ".acl", ".acl", "", false),
    ev("own-acl-percent-encoded", "%2Eacl", ".acl", "", true),
    ev("own-acl-fullwidth-dot", "\u{FF0E}acl", ".acl", "", true),
    // ── RESOURCE `.acl` shadowing the EXISTING document `d0.ttl` (a dotted name) ──────
    ev(
        "resource-acl-plain",
        "d0.ttl.acl",
        "d0.ttl.acl",
        "d0.ttl",
        false,
    ),
    // percent-encoded dot: `%2E`/`%2e` decode to `.`, so the raw ends in `Eacl` — a
    // naive exact-suffix guard mints it. GENUINE escalations.
    ev(
        "resource-acl-percent-upper",
        "d0.ttl%2Eacl",
        "d0.ttl.acl",
        "d0.ttl",
        true,
    ),
    ev(
        "resource-acl-percent-lower",
        "d0.ttl%2eacl",
        "d0.ttl.acl",
        "d0.ttl",
        true,
    ),
    // unicode compatibility dots (U+FF0E FULLWIDTH / U+2024 ONE DOT LEADER) fold to `.`
    // under NFKC; the raw codepoint before `acl` is not ASCII `.`, so a naive guard misses.
    ev(
        "resource-acl-fullwidth-dot",
        "d0.ttl\u{FF0E}acl",
        "d0.ttl.acl",
        "d0.ttl",
        true,
    ),
    ev(
        "resource-acl-one-dot-leader",
        "d0.ttl\u{2024}acl",
        "d0.ttl.acl",
        "d0.ttl",
        true,
    ),
    // trailing dot / space: Windows/filesystem canonicalisation strips them, so the stored
    // name becomes `d0.ttl.acl`, but the raw ends in `acl.` / `acl ` — a naive guard misses.
    ev(
        "resource-acl-trailing-dot",
        "d0.ttl.acl.",
        "d0.ttl.acl",
        "d0.ttl",
        true,
    ),
    ev(
        "resource-acl-trailing-space",
        "d0.ttl.acl ",
        "d0.ttl.acl",
        "d0.ttl",
        true,
    ),
];

// ─── the decode / acl-shape model (mirrors sparq-lws-core's mint-side guard intent) ───

/// Map the compatibility "dot-like" codepoints an NFKC normaliser folds onto ASCII `.`
/// (full-width full stop, one-dot leader, ideographic/small full stop) to `.`; other
/// characters pass through. Models the normalisation a hardened mint guard must apply
/// before its `.acl` check.
fn map_dotlike(c: char) -> char {
    match c {
        '\u{FF0E}' | '\u{2024}' | '\u{3002}' | '\u{FE52}' => '.',
        other => other,
    }
}

/// Canonicalise a raw POST child name to the *effective* name the resource actually
/// gets: percent-decode → compatibility-dot-map → strip trailing dots/spaces (the
/// filesystem/Windows canonicalisation an attacker exploits).
pub fn canonicalize_child(raw: &str) -> String {
    let decoded = sparq_core::percent::decode(raw, false);
    let mapped: String = decoded.chars().map(map_dotlike).collect();
    mapped.trim_end_matches(['.', ' ']).to_string()
}

/// Whether a name's final path segment names an `.acl` control document, matched
/// case-insensitively and tolerant of one trailing `/` (a container child). This is the
/// access-/mint-side predicate: `true` ⇒ the operation requires [`Mode::Control`].
pub fn is_acl_shaped(name: &str) -> bool {
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    let segment = trimmed.rsplit('/').next().unwrap_or(trimmed);
    segment.to_ascii_lowercase().ends_with(".acl")
}

/// What a *naive* (vulnerable) guard does: exact-case `ends_with(".acl")` on the RAW,
/// un-decoded slug. The whole point of the corpus is the inputs where this returns
/// `false` while [`is_acl_shaped`]`(canonicalize_child(raw))` is `true`.
pub fn naive_acl_guard(raw: &str) -> bool {
    raw.ends_with(".acl")
}

// ─── the decision-layer harness ───────────────────────────────────────────────────────

/// Build the escalation-target [`PodStore`] from the embedded [`WAC_NQUADS`] pod (the
/// same fixture the B1 vectors use) with the WAC materializer.
pub fn build_escalation_store() -> Result<PodStore, String> {
    let graph = Graph::load_dataset(WAC_NQUADS, "nquads")?;
    let mut store = PodStore::new(graph);
    store.materialize_wac()?;
    Ok(store)
}

/// A session for `agent` (anonymous when `None`), no client/issuer/time constraint.
fn session(agent: Option<&'static str>) -> Session<'static> {
    Session {
        agent,
        client: None,
        issuer: None,
        now: None,
    }
}

/// The non-`Control` principals whose escalation the corpus refutes: an authenticated
/// non-owner (bob), a second non-owner (carol), and an anonymous requester. NONE holds
/// `Control` over `priv0/` in [`WAC_NQUADS`].
pub fn non_control_principals() -> [Session<'static>; 3] {
    [session(Some(BOB)), session(Some(CAROL)), session(None)]
}

/// The IRI of the resource the minted `.acl` would GOVERN — the container [`PARENT`]
/// itself for the empty stem, otherwise `PARENT + governed_stem`. Authoring `<R>.acl` is
/// an `acl:Control` operation on this resource, so this is what the decision is taken over.
pub fn governed_resource(vector: &EscalationVector) -> String {
    if vector.governed_stem.is_empty() {
        PARENT.to_string()
    } else {
        format!("{PARENT}{}", vector.governed_stem)
    }
}

/// One corpus assertion that did not hold, with a human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscalationFailure {
    /// The [`EscalationVector::name`] of the failing entry.
    pub vector: &'static str,
    /// What failed (decode / acl-shape / naive-flag / a non-fail-closed decision).
    pub detail: String,
}

/// Evaluate one vector against `store`, returning every assertion that did not hold:
///
/// 1. `canonicalize_child(raw)` reproduces the declared `effective_child`.
/// 2. The effective name is exactly `<governed_stem>.acl` and is [`is_acl_shaped`] (so
///    `Control` is the required mode).
/// 3. The `raw_evades_naive_check` flag agrees with [`naive_acl_guard`].
/// 4. NON-VACUITY anchor: the owner (alice) DOES hold `Control` over the governed
///    resource — so the non-owner denial below is about the PRINCIPAL, not a resource
///    nobody can ever control.
/// 5. For EVERY non-`Control` principal, [`PodStore::decide`]`(…, governed, Control).allow`
///    equals the (fail-closed) `expect_allow` — the escalation is refused.
pub fn check_vector(store: &PodStore, vector: &EscalationVector) -> Vec<EscalationFailure> {
    let mut failures = Vec::new();
    let mut fail = |detail: String| {
        failures.push(EscalationFailure {
            vector: vector.name,
            detail,
        });
    };

    let canon = canonicalize_child(vector.raw_child);
    if canon != vector.effective_child {
        fail(format!(
            "canonicalize({:?}) = {:?}, expected effective {:?}",
            vector.raw_child, canon, vector.effective_child
        ));
    }
    if vector.effective_child != format!("{}.acl", vector.governed_stem) {
        fail(format!(
            "effective {:?} is not <governed_stem={:?}>.acl",
            vector.effective_child, vector.governed_stem
        ));
    }
    if !is_acl_shaped(vector.effective_child) {
        fail(format!(
            "effective {:?} is not .acl-shaped (Control would not be required)",
            vector.effective_child
        ));
    }
    if naive_acl_guard(vector.raw_child) == vector.raw_evades_naive_check {
        fail(format!(
            "raw_evades_naive_check={} disagrees with naive_acl_guard({:?})={}",
            vector.raw_evades_naive_check,
            vector.raw_child,
            naive_acl_guard(vector.raw_child)
        ));
    }

    let resource = governed_resource(vector);

    // NON-VACUITY anchor: the owner legitimately holds Control over the governed resource.
    let owner = session(Some(ALICE));
    if !store.decide(&owner, &resource, Mode::Control).allow {
        fail(format!(
            "vacuity: owner (alice) does NOT hold Control over governed {resource:?} — the \
             non-owner denial would be vacuous (use a materialized governed resource)"
        ));
    }

    // The escalation refutation: no non-Control requester obtains Control over the ACL's
    // governed resource (so none can author / shadow that control document).
    for principal in non_control_principals() {
        let decision = store.decide(&principal, &resource, Mode::Control);
        if decision.allow != vector.expect_allow {
            fail(format!(
                "escalation: decide(agent={:?}, {resource:?}, Control).allow = {}, expected {} \
                 (a non-Control requester must never author an .acl over this resource)",
                principal.agent, decision.allow, vector.expect_allow
            ));
        }
    }

    failures
}

/// Run the whole corpus against `store`; empty ⇔ every escalation is refused fail-closed.
pub fn run_corpus(store: &PodStore) -> Vec<EscalationFailure> {
    CORPUS
        .iter()
        .flat_map(|vector| check_vector(store, vector))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── the acceptance run: every escalation variant is fail-closed ───────────────

    #[test]
    fn escalation_corpus_all_fail_closed() {
        let store = build_escalation_store().expect("wac fixture builds");
        assert!(!CORPUS.is_empty(), "the corpus must be non-empty");
        let failures = run_corpus(&store);
        assert!(
            failures.is_empty(),
            "expected every escalation variant fail-closed, got: {failures:#?}"
        );
    }

    #[test]
    fn every_variant_decodes_to_an_acl_document() {
        // Independent of the store: each raw name canonicalises to its declared effective
        // form AND that form is .acl-shaped (so the future guard must require Control).
        for vector in CORPUS {
            assert_eq!(
                canonicalize_child(vector.raw_child),
                vector.effective_child,
                "{}: canonicalize mismatch",
                vector.name
            );
            assert!(
                is_acl_shaped(vector.effective_child),
                "{}: effective name is not .acl-shaped",
                vector.name
            );
        }
    }

    // ─── the COVER WITNESS (anti-vacuity): a genuine escalation a naive guard misses ─

    #[test]
    fn corpus_contains_a_genuine_escalation_that_a_naive_guard_misses() {
        let store = build_escalation_store().expect("wac fixture builds");
        let genuine: Vec<&EscalationVector> =
            CORPUS.iter().filter(|v| v.raw_evades_naive_check).collect();
        assert!(
            !genuine.is_empty(),
            "the corpus must contain >=1 GENUINE escalation attempt (raw evades a naive guard)"
        );
        for vector in genuine {
            // (a) genuinely adversarial: the raw form evades the naive exact-suffix guard,
            assert!(
                !naive_acl_guard(vector.raw_child),
                "{}: flagged as evading but the naive guard catches it",
                vector.name
            );
            // (b) yet decodes to a real .acl document,
            assert!(
                is_acl_shaped(&canonicalize_child(vector.raw_child)),
                "{}: evader must still decode to an .acl document",
                vector.name
            );
            // (c) and STILL survives the harness fail-closed (the interesting input is not
            //     silently pruned — it is checked, and the decision refuses it).
            assert!(
                check_vector(&store, vector).is_empty(),
                "{}: a genuine escalation must be refused fail-closed",
                vector.name
            );
        }
    }

    // ─── NON-VACUITY spot-check: flipping the expected verdict makes the harness fail ─

    #[test]
    fn flipping_expected_allow_to_true_makes_the_harness_report_a_failure() {
        let store = build_escalation_store().expect("wac fixture builds");
        // Pick a concrete genuine escalation and confirm the un-mutated check passes …
        let base = CORPUS
            .iter()
            .find(|v| v.raw_evades_naive_check)
            .expect("a genuine escalation exists");
        assert!(
            check_vector(&store, base).is_empty(),
            "precondition: the fail-closed expectation holds on the real engine"
        );
        // … then flip the expected decision to `allow` — the store still (correctly)
        // denies, so the assertion mismatch MUST surface as a failure. This proves the
        // harness cannot pass vacuously: it is genuinely comparing against the engine.
        let mut mutant = *base;
        mutant.expect_allow = true;
        let failures = check_vector(&store, &mutant);
        assert!(
            failures.iter().any(|f| f.detail.contains("escalation:")),
            "flipping expect_allow → true must produce an escalation failure, got {failures:#?}"
        );
    }

    // ─── direct unit tests per public fn (coverage ratchet) ────────────────────────

    #[test]
    fn canonicalize_percent_decodes_and_normalises() {
        assert_eq!(canonicalize_child("%2Eacl"), ".acl");
        assert_eq!(canonicalize_child("x%2eacl"), "x.acl");
        assert_eq!(canonicalize_child("x\u{FF0E}acl"), "x.acl");
        assert_eq!(canonicalize_child("x.acl."), "x.acl");
        assert_eq!(canonicalize_child("x.acl  "), "x.acl");
        // A benign name is untouched.
        assert_eq!(canonicalize_child("note.ttl"), "note.ttl");
        // A malformed escape is passed through, not silently dropped.
        assert_eq!(canonicalize_child("a%zz"), "a%zz");
    }

    #[test]
    fn is_acl_shaped_matches_the_final_segment_case_insensitively() {
        assert!(is_acl_shaped("x.acl"));
        assert!(is_acl_shaped(".acl"));
        assert!(is_acl_shaped("secret.ACL"));
        assert!(is_acl_shaped("https://pod.ex/priv0/foo.acl/")); // container child
        assert!(!is_acl_shaped("x.ttl"));
        assert!(!is_acl_shaped("aclremap"));
        assert!(!is_acl_shaped("https://x.acl/child")); // .acl only mid-path
    }

    #[test]
    fn naive_guard_misses_encoded_and_unicode_but_catches_plain() {
        assert!(naive_acl_guard("x.acl"));
        assert!(naive_acl_guard(".acl"));
        assert!(!naive_acl_guard("x%2Eacl"));
        assert!(!naive_acl_guard("x\u{FF0E}acl"));
        assert!(!naive_acl_guard("x.acl."));
    }

    #[test]
    fn governed_resource_targets_the_container_or_the_named_resource() {
        let own = ev("own", ".acl", ".acl", "", false);
        assert_eq!(governed_resource(&own), "https://pod.ex/priv0/");
        let res = ev("res", "d0.ttl.acl", "d0.ttl.acl", "d0.ttl", false);
        assert_eq!(governed_resource(&res), "https://pod.ex/priv0/d0.ttl");
    }

    #[test]
    fn owner_alice_holds_control_over_every_governed_resource() {
        // The non-vacuity anchor, surfaced as its own test: for EVERY corpus entry the
        // pod owner (alice) genuinely DOES hold Control over the governed resource, so the
        // non-owner denials are about the PRINCIPAL — not a resource nobody can control.
        let store = build_escalation_store().expect("wac fixture builds");
        let alice = session(Some(ALICE));
        for vector in CORPUS {
            let resource = governed_resource(vector);
            assert!(
                store.decide(&alice, &resource, Mode::Control).allow,
                "{}: owner alice must hold Control over {resource:?}",
                vector.name
            );
        }
    }
}

// ─── DOMAIN-COVERAGE SELF-CHECK (sq-og8u8 anti-vacuity, research §5.1) ────────────────
//
// A `#[cfg(kani)]` proof pinning that the corpus genuinely contains a surviving
// escalation. The exact-image asserts over the static `CORPUS` use no symbolic loop that
// could itself be pruned; the `kani::cover!` proves a symbolic index REACHES a genuine
// escalation under the bound, so a future re-scope that empties the interesting inputs
// (the sq-sqtk2.1 failure mode) goes RED instead of passing vacuously.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Allocation-free PROOF-ONLY equivalent of [`is_acl_shaped`] for ASCII input:
    /// same semantics (drop one trailing `/`, take the final `/`-segment, case-fold,
    /// `.acl` suffix) but case-folds the last four bytes in place instead of calling
    /// `to_ascii_lowercase()`, whose heap-allocating `String` blows CBMC's string/heap
    /// model past the harness wall-clock budget. The proof asserts `is_ascii()` on every
    /// input it classifies, under which the two predicates coincide byte-for-byte, so
    /// the verified property is UNCHANGED. Production paths still use [`is_acl_shaped`].
    fn is_acl_shaped_ascii_noalloc(name: &str) -> bool {
        let bytes = name.as_bytes();
        // Tolerate one trailing '/' (a container child), as is_acl_shaped does.
        let mut end = bytes.len();
        if end > 0 && bytes[end - 1] == b'/' {
            end -= 1;
        }
        // Start of the final path segment.
        let mut start = 0;
        let mut j = 0;
        while j < end {
            if bytes[j] == b'/' {
                start = j + 1;
            }
            j += 1;
        }
        if end - start < 4 {
            return false;
        }
        bytes[end - 4] == b'.'
            && bytes[end - 3].to_ascii_lowercase() == b'a'
            && bytes[end - 2].to_ascii_lowercase() == b'c'
            && bytes[end - 1].to_ascii_lowercase() == b'l'
    }

    #[kani::proof]
    #[kani::unwind(20)]
    fn domain_corpus_exhibits_a_surviving_escalation() {
        // Exact-image: at least one genuine escalation is present AND adversarial.
        let mut found = false;
        let mut i = 0;
        while i < CORPUS.len() {
            let entry = &CORPUS[i];
            if entry.raw_evades_naive_check {
                assert!(
                    !naive_acl_guard(entry.raw_child),
                    "a flagged evader must actually evade the naive exact-suffix guard"
                );
                assert!(
                    entry.effective_child.is_ascii(),
                    "corpus effective names are ASCII (precondition for the \
                     allocation-free .acl classification below)"
                );
                assert!(
                    is_acl_shaped_ascii_noalloc(entry.effective_child),
                    "a flagged evader must still decode to an .acl control document"
                );
                assert!(
                    !entry.expect_allow,
                    "every escalation entry must expect a fail-closed (deny) decision"
                );
                found = true;
            }
            i += 1;
        }
        assert!(
            found,
            "the corpus MUST contain >=1 genuine escalation attempt (else it is vacuous)"
        );

        // Witness survival: a symbolic index reaches a genuine escalation under `unwind`.
        let idx: usize = kani::any();
        kani::assume(idx < CORPUS.len());
        kani::cover!(
            CORPUS[idx].raw_evades_naive_check,
            "a genuine escalation index is reachable under the harness bound"
        );
    }
}
