//! # `secprop` — the per-protocol security-properties annotation graph (§5c)
//!
//! The **machine-readable form** of the trust model each sparq-mpc protocol module
//! states in its own docs: a static annotation graph
//! (`ontologies/secprop-mpc-methods.ttl`) keyed on a `mpc:` protocol IRI, expressed
//! with the `secx:` extension vocabulary of the zero-dependency
//! [`sparq_secprop_vocab`] leaf (issues #3275 + #6186; design record
//! `research/security-properties-ontology-design.md` §5c).
//!
//! ## Why a LEVEL and not just an assumption (#6186)
//!
//! [`sparq_secprop_vocab::SECX_ASSUMPTION`] is human-facing **provenance**: the
//! shared N3 admissibility reasoner (`sparq_trust::admissibility`) keys strictly on
//! `(dimension, level)` through `secx:overDimension` + `secx:atLeast`, so an
//! `secx:assumption secx:SemiHonest` triple alone can never be discharged by an ODRL
//! preference. Each block in the graph therefore carries **both**: the `secx:level`
//! on one of the two ordered MPC dimensions
//! ([`sparq_secprop_vocab::SECX_ADVERSARY_MODEL`],
//! [`sparq_secprop_vocab::SECX_CORRUPTION_THRESHOLD`]) that the reasoner reads, and
//! the `secx:assumption` a human reads.
//!
//! The exclusion itself is **not** recomputed here. [`annotations_n3`] hands the
//! graph to `sparq_trust::admissible`, which is the one reasoner: a preference
//! `requiresAdversaryModel gteq secx:Malicious` or `requiresCorruptionThreshold gteq
//! secx:DishonestMajority` excludes every protocol below, and a relaxed `gteq
//! secx:SemiHonest` / `gteq secx:HonestMajority` admits them. This crate ships no
//! second, crate-local exclusion predicate.
//!
//! ## What this records — and what it is NOT
//!
//! It **records** a (protocol, dimension) → (level, assurance, audit-status,
//! assumption) claim and its epistemic basis; it is **NOT** a proof of any property.
//! `sparq-mpc` is a research scaffold and is **not externally audited** (`sq-qhy4`),
//! so every assertion is at most `secx:Claimed` with
//! `secx:ExternalSignOffPending`.
//!
//! **No sparq-mpc protocol is annotated `secx:Malicious`**, including the IT-MAC
//! modules: their hardening is real but partial, as [`crate::auth_disclose`]'s own
//! docs state ("Integrity coverage is NOT total"). None is annotated `secx:Covert` either: covert security
//! means a stated deterrence probability, which no module claims.
//!
//! ## Opt-in by construction
//!
//! Behind the **default-OFF `secprop-annotations`** cargo feature. Its only
//! dependency is the `sparq-secprop-vocab` leaf, which has none of its own, so the
//! opt-in build grows by a `const &str` table and the Turtle; the default build is
//! byte-identical with it off.

/// The `mpc:` namespace the annotated protocol IRIs live under.
pub const MPC_NS: &str = "https://sparq.dev/ns/mpc#";

/// `mpc:shamir-honest-majority-v1` — the Shamir `t`-of-`n` backend
/// ([`crate::shamir::ShamirBackend`]).
pub const SHAMIR_HONEST_MAJORITY_V1: &str = "https://sparq.dev/ns/mpc#shamir-honest-majority-v1";
/// `mpc:secure-compare-v1` — the semi-honest secure comparison / threshold
/// ([`crate::compare`]).
pub const SECURE_COMPARE_V1: &str = "https://sparq.dev/ns/mpc#secure-compare-v1";
/// `mpc:hidden-value-join-v1` — the hidden-value join / circuit-PSI core
/// ([`crate::join::HiddenValueJoin`] and its batched / fully-oblivious tiers).
pub const HIDDEN_VALUE_JOIN_V1: &str = "https://sparq.dev/ns/mpc#hidden-value-join-v1";
/// `mpc:auth-compare-v1` — the IT-MAC comparison chain ([`crate::auth_compare`]).
pub const AUTH_COMPARE_V1: &str = "https://sparq.dev/ns/mpc#auth-compare-v1";
/// `mpc:auth-disclose-threshold-v1` — the experimental IT-MAC threshold disclosure
/// ([`crate::auth_disclose`]).
pub const AUTH_DISCLOSE_THRESHOLD_V1: &str = "https://sparq.dev/ns/mpc#auth-disclose-threshold-v1";
/// `mpc:federated-pipeline-v1` — the end-to-end federated driver
/// ([`crate::pipeline`]).
pub const FEDERATED_PIPELINE_V1: &str = "https://sparq.dev/ns/mpc#federated-pipeline-v1";
/// Every **sparq-mpc protocol** the annotation graph covers, in declaration order.
/// The graph has no other subject: [`annotations_n3`] describes implemented
/// protocols only.
pub const ANNOTATED_PROTOCOLS: &[&str] = &[
    SHAMIR_HONEST_MAJORITY_V1,
    SECURE_COMPARE_V1,
    HIDDEN_VALUE_JOIN_V1,
    AUTH_COMPARE_V1,
    AUTH_DISCLOSE_THRESHOLD_V1,
    FEDERATED_PIPELINE_V1,
];

/// The annotation graph as a self-contained Turtle/N3 document (its own `@prefix`
/// declarations included).
///
/// This is the exact text [`annotations_n3`] hands to the shared admissibility
/// reasoner; it is a compiled-in constant, so there is no I/O and no file to go
/// missing at runtime.
const METHODS_TTL: &str = include_str!("../ontologies/secprop-mpc-methods.ttl");

/// The per-protocol annotation graph, ready to pass as the `annotations` argument of
/// `sparq_trust::admissible` (behind that crate's `secprop-admissibility` feature).
///
/// The document is self-contained: it redeclares the `mpc:`/`secx:`/`rdfs:` prefixes
/// it uses, so it composes with a reasoner input that already carries its own
/// preamble (a Turtle/N3 `@prefix` may appear anywhere above its first use, and
/// redeclaring a prefix to the same IRI is a no-op).
///
/// ```
/// let g = sparq_mpc::secprop::annotations_n3();
/// assert!(g.contains(sparq_mpc::secprop::SHAMIR_HONEST_MAJORITY_V1.rsplit('#').next().unwrap()));
/// ```
#[must_use]
pub fn annotations_n3() -> &'static str {
    METHODS_TTL
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_secprop_vocab::{
        ALL_SECPROP_IRIS, SECX_ADVERSARY_MODEL, SECX_ASSURANCE_LEVEL, SECX_CORRUPTION_THRESHOLD,
        SECX_COVERT, SECX_HONEST_MAJORITY, SECX_MALICIOUS, SECX_PROVEN, SECX_SEMI_HONEST,
        SEC_PROP_NS,
    };

    /// `annotations_n3` returns the compiled-in graph (the one public fn here).
    #[test]
    fn annotations_n3_returns_the_bundled_graph() {
        let g = annotations_n3();
        assert!(g.starts_with("# secprop-mpc-methods.ttl"));
        assert!(
            g.contains("@prefix mpc:"),
            "the graph must be self-contained"
        );
        assert!(
            g.contains("@prefix secx:"),
            "the graph must be self-contained"
        );
    }

    /// Every protocol in [`ANNOTATED_PROTOCOLS`] is a subject of the graph, annotated
    /// on BOTH ordered MPC dimensions and with a method-wide `secx:AssuranceLevel`
    /// — the completeness rule. A protocol with only
    /// one of the two axes would be silently admitted by a preference over the other.
    #[test]
    fn every_protocol_is_annotated_on_both_dimensions() {
        let g = annotations_n3();
        for &p in ANNOTATED_PROTOCOLS {
            let local = p
                .strip_prefix(MPC_NS)
                .expect("protocol IRI is in the mpc: namespace");
            let block = g
                .split(&format!("\nmpc:{}\n", local))
                .nth(1)
                .unwrap_or_else(|| panic!("no annotation block for mpc:{}", local));
            // The block runs to the first blank line after its terminating ` .`.
            let block = block.split("\n\n").next().unwrap();
            for dim in [
                SECX_ADVERSARY_MODEL,
                SECX_CORRUPTION_THRESHOLD,
                SECX_ASSURANCE_LEVEL,
            ] {
                let local_dim = dim.strip_prefix(SEC_PROP_NS).unwrap();
                assert!(
                    block.contains(&format!("secx:property secx:{} ", local_dim)),
                    "mpc:{} is not annotated on secx:{} — a preference over that \
                     dimension could not be discharged for it",
                    local,
                    local_dim,
                );
            }
        }
    }

    /// HONESTY RULE 1 (load-bearing): no sparq-mpc protocol claims `secx:Malicious`
    /// or `secx:Covert` on the adversary-model axis, and none claims a corruption
    /// threshold stronger than `secx:HonestMajority`. The IT-MAC modules' hardening
    /// is real but partial (see `crate::auth_disclose`'s module docs),
    /// so the level stays `secx:SemiHonest` and the hardening is recorded in prose.
    #[test]
    fn no_sparq_protocol_overclaims_its_trust_model() {
        let g = annotations_n3();
        for &p in ANNOTATED_PROTOCOLS {
            let local = p.strip_prefix(MPC_NS).unwrap();
            let block = g
                .split(&format!("\nmpc:{}\n", local))
                .nth(1)
                .unwrap()
                .split("\n\n")
                .next()
                .unwrap();
            for over in [SECX_MALICIOUS, SECX_COVERT] {
                let l = over.strip_prefix(SEC_PROP_NS).unwrap();
                assert!(
                    !block.contains(&format!("secx:level secx:{} ", l)),
                    "mpc:{} claims secx:{} — sparq-mpc delivers neither (see the \
                     honesty rules in secprop-mpc-methods.ttl)",
                    local,
                    l,
                );
            }
            assert!(
                block.contains(&format!(
                    "secx:level secx:{} ",
                    SECX_SEMI_HONEST.strip_prefix(SEC_PROP_NS).unwrap()
                )),
                "mpc:{} must hold the SemiHonest adversary-model level",
                local,
            );
            assert!(
                block.contains(&format!(
                    "secx:level secx:{} ",
                    SECX_HONEST_MAJORITY.strip_prefix(SEC_PROP_NS).unwrap()
                )),
                "mpc:{} must hold the HonestMajority corruption-threshold level",
                local,
            );
            // And no `secx:Proven` anywhere while sq-qhy4 is open.
            assert!(
                !block.contains(SECX_PROVEN.strip_prefix(SEC_PROP_NS).unwrap()),
                "mpc:{} may not carry secx:Proven while sq-qhy4 is open",
                local,
            );
        }
    }

    /// The shipped graph's subjects are EXACTLY [`ANNOTATED_PROTOCOLS`]: no
    /// illustrative or unimplemented subject can reach the admissibility reasoner
    /// through [`annotations_n3`] (a positive control lives in a test fixture).
    #[test]
    fn annotations_n3_names_only_implemented_protocols() {
        let subjects: Vec<String> = annotations_n3()
            .lines()
            .filter_map(|l| l.strip_prefix("mpc:"))
            .map(|local| format!("{}{}", MPC_NS, local.trim_end()))
            .collect();
        let want: Vec<String> = ANNOTATED_PROTOCOLS.iter().map(|p| p.to_string()).collect();
        assert_eq!(subjects, want);
        assert!(!annotations_n3().to_lowercase().contains("illustrative"));
    }

    /// Dimension/level IRI drift guard: every `secx:` term the graph references is a
    /// term of the canonical vocabulary registry (pinned to `secprop-ext.ttl` by the
    /// leaf's own drift test). A typo'd or renamed level fails here rather than
    /// silently never discharging.
    #[test]
    fn graph_secx_terms_are_canonical_vocabulary_terms() {
        let canonical: std::collections::HashSet<&str> = ALL_SECPROP_IRIS
            .iter()
            .filter_map(|iri| iri.strip_prefix(SEC_PROP_NS))
            .collect();
        let mut seen = 0usize;
        for line in annotations_n3().lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            for tok in line.split_whitespace() {
                let Some(local) = tok.strip_prefix("secx:") else {
                    continue;
                };
                let local = local.trim_end_matches([';', ',', '.', ']']);
                if local.is_empty() {
                    continue;
                }
                assert!(
                    canonical.contains(local),
                    "the mpc annotation graph references `secx:{}`, which is not a \
                     term of the canonical secx: vocabulary \
                     (`sparq_secprop_vocab::ALL_SECPROP_IRIS`) — IRI drift (#6186)",
                    local,
                );
                seen += 1;
            }
        }
        assert!(seen > 0, "no secx: terms scanned — parser/format drift");
    }
}
