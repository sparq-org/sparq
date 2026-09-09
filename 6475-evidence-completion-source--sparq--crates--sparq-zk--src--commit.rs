//! Per-named-graph commitments (plan §2.2, flat-hash primary shape).
//!
//! Pipeline: RDFC10-canonicalize the graph's content ([`crate::canon`]) →
//! encode each canonical triple to a leaf ([`crate::encode`]) in canonical
//! N-Quads order (index = leaf index) → `C(G)` = one Poseidon2 sponge over
//! the leaf sequence — the same single `Poseidon2::hash(leaves, n)` call a
//! Noir circuit makes to recompute the commitment in-circuit (the
//! length-bearing IV gives domain separation per leaf count; runtime
//! `message_size` < the bucket's `n_max` handles padding slots).
//!
//! The per-graph Merkle fallback for large graphs (plan §2.2 shape 2) is a
//! later deliverable; this module is the credential-scale primary path.

use crate::canon::{self, CanonError, CanonicalGraph};
use crate::encode;
use crate::field::Fr;
use crate::poseidon2;
use oxrdf::Triple;
use std::collections::HashMap;

// [OPUS-4.8] sq-zzxt: the commitment-method config registry.
//
// This is the config-plumbing layer of the maintainer-greenlit (#769)
// configurable-commitment build-out (design: `research/zk-configurable-commitment-design.md`,
// §2 / §10; foundation: `research/zk-field-native-encoding.md`, #794). It selects
// HOW a graph was committed, recorded as a distinct `zk:scheme` IRI on the
// registry entry, so a verifier can fail closed on an unknown/mismatched method.
//
// SCOPE: config ONLY. This module introduces NO circuit, NO leaf-shape change,
// and NO dual-leaf/value-only encoding — those are separate, audit-gated beads
// (`sq-j506` dual-leaf host encoding, `sq-cfmv` per-method circuit dispatch). The
// existing `string-canonical` pipeline (`encode.rs`/`commit.rs`) is byte-unchanged
// and stays the default. The method axis is the single source of truth that a
// later `(method, circuit)` dispatch matrix (sq-cfmv) will consume.
//
// HONESTY (load-bearing — `research/zk-configurable-commitment-design.md` §0/§7,
// `research/zk-field-native-encoding.md` §5): the dual-leaf method carries the
// INV-VL downgrade — the in-circuit invariant "the compared value equals the
// parse of the committed lexical bytes" is REMOVED for the value-FILTER lane,
// so value<->lexical agreement migrates from a machine-enforced circuit property
// to TRUSTED-ISSUER-HONESTY (#769 accepted at research grade; gap CR-G8). The
// whole ZK estate is remediated + internally re-audited but NOT externally
// audited (sq-qhy4, P0); nothing here is a soundness or privacy guarantee.

/// How a named graph was committed — the selectable commitment method over the
/// existing `zk:scheme` registry slot ([`crate::registry::RegistryEntry::scheme`]).
///
/// This is a **closed enum, by design** (`research/zk-configurable-commitment-design.md`
/// §10 default (1)): the method is **in-circuit-affecting** (it determines which
/// leaf shape a graph was committed under and therefore which circuit family is
/// legal against it — sq-cfmv), so it must **fail closed** on an unknown method
/// rather than admit an open trait an attacker could extend. An unrecognized
/// `zk:scheme` IRI maps to `None`, never to a default (see [`Self::from_scheme_iri`]).
///
/// The method is chosen **at ingest** (issuance time) and is **immutable for that
/// graph's commitment**: a graph committed under one method's leaf shape cannot
/// later be proven with a circuit that expects a different shape.
///
/// # Per-method honesty (NOT a guarantee)
/// - [`StringCanonicalV1`](Self::StringCanonicalV1): the conservative default and
///   back-compat anchor — the only method implemented end-to-end today. The
///   in-circuit INV-VL invariant holds for its value-FILTER lane.
/// - [`DualLeafV1`](Self::DualLeafV1): the value-optimised method for new issuance
///   (#769). It **REMOVES INV-VL** — value<->lexical agreement rests on trusted-
///   issuer honesty (machine-enforced -> issuer-honesty downgrade), accepted at
///   research grade per #769 and registered as an **open external-audit obligation**
///   (gap CR-G8 / sq-qhy4). The leaf encoding itself is NOT implemented here (config
///   only; impl is sq-j506).
/// - `ValueOnlyV1` (feature `commitment-value-only` only): a **benchmark / research
///   dial**, never a production default (§10 default (2)). It loses term identity
///   entirely (no lexical component). It is **compiled out by default**; the feature
///   name is the warning. See the feature-gated variant below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitmentMethod {
    /// (i) `string-canonical` (`zk:poseidon2-rdfc10-v1`): blake3 over the canonical
    /// N-Triples lexical token only; the in-circuit INV-VL invariant is
    /// machine-enforced for the value-FILTER lane. **Today's implemented default.**
    StringCanonicalV1,
    /// (iii) `dual-leaf` (`zk:poseidon2-dualleaf-v1`): a value handle AND a
    /// lexical-identity hash in one leaf. **REMOVES INV-VL** (value<->lexical
    /// agreement becomes trusted-issuer-honesty, #769 accepted at research grade;
    /// CR-G8 / sq-qhy4). Config-only here; the leaf encoding is sq-j506 (NOT
    /// implemented). [OPUS-4.8]
    DualLeafV1,
    /// (ii) `value-only` (`zk:poseidon2-valuehook-v1`): a single value-first leaf;
    /// the lexical hash is **dropped**, so term identity is lost entirely. A
    /// **benchmark / research comparison dial only** — never select for real
    /// issuance (`research/zk-configurable-commitment-design.md` §2.2, §10 default
    /// (2)). Gated behind the OFF-by-default `commitment-value-only` feature so a
    /// normal build cannot select it. [OPUS-4.8]
    #[cfg(feature = "commitment-value-only")]
    ValueOnlyV1,
}

impl CommitmentMethod {
    /// The byte-unchanged, back-compat default ([`Self::StringCanonicalV1`]).
    /// `value-only` is NEVER a default (§10 default (2)); `dual-leaf` is opt-in per
    /// graph at ingest, not a default.
    pub const DEFAULT: CommitmentMethod = CommitmentMethod::StringCanonicalV1;

    /// The `zk:scheme` IRI that records this method on a [`RegistryEntry`].
    ///
    /// [`RegistryEntry`]: crate::registry::RegistryEntry
    pub const fn scheme_iri(self) -> &'static str {
        match self {
            CommitmentMethod::StringCanonicalV1 => {
                crate::registry::ZK_SCHEME_POSEIDON2_RDFC10_V1
            }
            CommitmentMethod::DualLeafV1 => crate::registry::ZK_SCHEME_POSEIDON2_DUALLEAF_V1,
            #[cfg(feature = "commitment-value-only")]
            CommitmentMethod::ValueOnlyV1 => crate::registry::ZK_SCHEME_POSEIDON2_VALUEHOOK_V1,
        }
    }

    /// Parse a method from its `zk:scheme` IRI. **Fail-closed**: an unknown or
    /// unrecognized IRI returns `None` (never a default) — the closed-enum
    /// discipline that lets a verifier reject a graph committed under a method it
    /// does not understand (§10 default (1)). When the `commitment-value-only`
    /// feature is OFF, the value-only IRI is unknown and rejected, so a build that
    /// did not opt into the research dial cannot be tricked into selecting it.
    pub fn from_scheme_iri(iri: &str) -> Option<CommitmentMethod> {
        match iri {
            crate::registry::ZK_SCHEME_POSEIDON2_RDFC10_V1 => {
                Some(CommitmentMethod::StringCanonicalV1)
            }
            crate::registry::ZK_SCHEME_POSEIDON2_DUALLEAF_V1 => Some(CommitmentMethod::DualLeafV1),
            #[cfg(feature = "commitment-value-only")]
            crate::registry::ZK_SCHEME_POSEIDON2_VALUEHOOK_V1 => {
                Some(CommitmentMethod::ValueOnlyV1)
            }
            _ => None,
        }
    }

    /// The `zk:scheme` IRI as an [`oxrdf::NamedNode`], for writing a method onto a
    /// [`RegistryEntry`].
    ///
    /// [`RegistryEntry`]: crate::registry::RegistryEntry
    pub fn scheme_node(self) -> oxrdf::NamedNode {
        oxrdf::NamedNode::new_unchecked(self.scheme_iri())
    }

    /// Whether this method is sound to select for **production / real issuance** at
    /// the config layer. Honest, NOT a guarantee: `string-canonical` is the
    /// conservative anchor; `dual-leaf` carries the #769-accepted INV-VL downgrade
    /// (so `true` here only means "not a research-only dial", not "audited" — the
    /// estate is NOT externally audited, sq-qhy4); `value-only` is a research dial
    /// and returns `false`. Callers that refuse research-only methods can gate on
    /// this; it does **not** assert any cryptographic property.
    pub const fn is_production_selectable(self) -> bool {
        match self {
            CommitmentMethod::StringCanonicalV1 | CommitmentMethod::DualLeafV1 => true,
            #[cfg(feature = "commitment-value-only")]
            CommitmentMethod::ValueOnlyV1 => false,
        }
    }

    /// Whether this method removes the in-circuit INV-VL invariant (value =
    /// parse(committed lexical)) for the value-FILTER lane — i.e. whether
    /// value<->lexical agreement rests on trusted-issuer honesty rather than a
    /// machine-enforced circuit property. Honest per-method statement, NOT a
    /// guarantee: `dual-leaf` and `value-only` remove it (CR-G8 / sq-qhy4, #769
    /// accepted at research grade); `string-canonical` retains it. See the module
    /// doc and `research/zk-field-native-encoding.md` §5.
    pub const fn removes_inv_vl(self) -> bool {
        match self {
            CommitmentMethod::StringCanonicalV1 => false,
            CommitmentMethod::DualLeafV1 => true,
            #[cfg(feature = "commitment-value-only")]
            CommitmentMethod::ValueOnlyV1 => true,
        }
    }
}

impl Default for CommitmentMethod {
    fn default() -> Self {
        CommitmentMethod::DEFAULT
    }
}

/// Commitment failure.
#[derive(Debug)]
pub enum CommitError {
    Canon(CanonError),
    /// A canonical triple contained a term outside the committed data model.
    UncommittableTerm(String),
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitError::Canon(e) => write!(f, "{e}"),
            CommitError::UncommittableTerm(t) => write!(f, "uncommittable term: {t}"),
        }
    }
}

impl std::error::Error for CommitError {}

impl From<CanonError> for CommitError {
    fn from(e: CanonError) -> Self {
        CommitError::Canon(e)
    }
}

/// A committed graph: canonical form, ordered leaves, and the commitment.
#[derive(Debug, Clone)]
pub struct GraphCommitment {
    /// The RDFC10 canonical form (leaf order = `canonical.lines` order).
    pub canonical: CanonicalGraph,
    /// Per-triple leaf hashes, in canonical order.
    pub leaves: Vec<Fr>,
    /// `C(G)`: Poseidon2 sponge over `leaves`.
    pub commitment: Fr,
    /// The per-graph salt the leaves were encoded under (`zk:rdfc10Salt`).
    pub salt: Fr,
}

impl GraphCommitment {
    /// Leaf index of a canonical-form triple (bnode labels must be the
    /// RDFC10 canonical labels). `None` if the triple is not in the graph.
    pub fn leaf_index(&self, canonical_triple: &Triple) -> Option<usize> {
        self.canonical.triples.iter().position(|t| t == canonical_triple)
    }

    /// An index from canonical triple to leaf position, for bulk resolution
    /// (the zk-trace seam resolves many lookups per query).
    pub fn leaf_index_map(&self) -> HashMap<&Triple, usize> {
        self.canonical
            .triples
            .iter()
            .enumerate()
            .map(|(i, t)| (t, i))
            .collect()
    }
}

/// Canonicalizes and commits one named graph's content under `salt`.
pub fn commit_triples(triples: &[Triple], salt: Fr) -> Result<GraphCommitment, CommitError> {
    let canonical = canon::canonicalize_triples(triples)?;
    commit_canonical(canonical, salt)
}

/// Commits the content of a `sparq_core::Graph` under `salt`.
pub fn commit_graph_content(g: &sparq_core::Graph, salt: Fr) -> Result<GraphCommitment, CommitError> {
    let canonical = canon::canonicalize_graph_content(g)?;
    commit_canonical(canonical, salt)
}

fn commit_canonical(canonical: CanonicalGraph, salt: Fr) -> Result<GraphCommitment, CommitError> {
    let mut leaves = Vec::with_capacity(canonical.triples.len());
    for t in &canonical.triples {
        let leaf = encode::encode_triple(t, &salt)
            .ok_or_else(|| CommitError::UncommittableTerm(t.to_string()))?;
        leaves.push(leaf);
    }
    let commitment = poseidon2::hash(&leaves);
    Ok(GraphCommitment { canonical, leaves, commitment, salt })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::salt_from_bytes;
    use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term};

    fn bnode_graph(label: &str, value: &str) -> Vec<Triple> {
        let b = BlankNode::new(label).unwrap();
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(b.clone()),
                NamedNode::new("http://ex/p").unwrap(),
                Term::Literal(Literal::new_simple_literal(value)),
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(b),
                NamedNode::new("http://ex/q").unwrap(),
                Term::NamedNode(NamedNode::new("http://ex/o").unwrap()),
            ),
        ]
    }

    #[test]
    fn deterministic_and_input_order_independent() {
        let salt = salt_from_bytes(&[7u8; 32]);
        let g = bnode_graph("x", "v");
        let mut rev = g.clone();
        rev.reverse();
        let c1 = commit_triples(&g, salt).unwrap();
        let c2 = commit_triples(&rev, salt).unwrap();
        assert_eq!(c1.commitment, c2.commitment);
        assert_eq!(c1.leaves, c2.leaves);
    }

    #[test]
    fn salt_separation_same_content_different_graphs() {
        // The Q6 property: identical canonical content (same canonical bnode
        // labels) committed in two different graphs (different salts) must
        // produce different LEAVES, not just different commitments.
        let g = bnode_graph("x", "v");
        let c1 = commit_triples(&g, salt_from_bytes(&[1u8; 32])).unwrap();
        let c2 = commit_triples(&g, salt_from_bytes(&[2u8; 32])).unwrap();
        assert_eq!(c1.canonical.lines, c2.canonical.lines, "canonical form is salt-free");
        for (l1, l2) in c1.leaves.iter().zip(&c2.leaves) {
            assert_ne!(l1, l2, "every bnode-bearing leaf must be salt-separated");
        }
        assert_ne!(c1.commitment, c2.commitment);
    }

    // [OPUS-4.8] sq-zzxt: the commitment-method config registry.

    #[test]
    fn method_default_is_string_canonical() {
        // §10 default: string-canonical is THE default (back-compat anchor); the
        // byte-unchanged pipeline. Neither dual-leaf nor value-only is ever default.
        assert_eq!(CommitmentMethod::default(), CommitmentMethod::StringCanonicalV1);
        assert_eq!(CommitmentMethod::DEFAULT, CommitmentMethod::StringCanonicalV1);
    }

    #[test]
    fn method_scheme_iri_round_trips() {
        // parse(serialize(m)) == m for every method present in this build.
        for m in commitment_methods_in_build() {
            let iri = m.scheme_iri();
            assert_eq!(
                CommitmentMethod::from_scheme_iri(iri),
                Some(m),
                "scheme IRI must round-trip for {:?}",
                m
            );
            // The NamedNode serialization carries the same IRI.
            assert_eq!(m.scheme_node().as_str(), iri);
        }
    }

    #[test]
    fn method_scheme_iris_are_distinct() {
        // Each method MUST carry a DISTINCT zk:scheme IRI so a registry entry
        // records exactly how it was committed and a verifier can fail closed on
        // a mismatch (design §2.1).
        let iris: Vec<&str> = commitment_methods_in_build().iter().map(|m| m.scheme_iri()).collect();
        let mut sorted = iris.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), iris.len(), "scheme IRIs must be pairwise distinct");
    }

    #[test]
    fn method_string_canonical_iri_is_unchanged() {
        // The existing string-canonical scheme IRI is retained byte-for-byte
        // (back-compat: every already-issued credential records this IRI).
        assert_eq!(
            CommitmentMethod::StringCanonicalV1.scheme_iri(),
            "https://sparq.dev/ns/zk#poseidon2-rdfc10-v1"
        );
        // And it is precisely the legacy const the registry already used.
        assert_eq!(
            CommitmentMethod::StringCanonicalV1.scheme_iri(),
            crate::registry::ZK_SCHEME_POSEIDON2_RDFC10_V1
        );
    }

    #[test]
    fn method_parse_is_fail_closed_on_unknown() {
        // §10 default (1): an unknown / unrecognized zk:scheme IRI returns None,
        // NEVER a default — the closed-enum, fail-closed discipline.
        assert_eq!(CommitmentMethod::from_scheme_iri("https://sparq.dev/ns/zk#poseidon2-bogus"), None);
        assert_eq!(CommitmentMethod::from_scheme_iri(""), None);
        assert_eq!(CommitmentMethod::from_scheme_iri("not-an-iri"), None);
        // The signature cryptosuite IRI is a DIFFERENT axis — not a commitment method.
        assert_eq!(
            CommitmentMethod::from_scheme_iri(crate::registry::ZK_SCHEME_POSEIDON2_RDFC10_V1.replace("rdfc10", "schnorr").as_str()),
            None
        );
    }

    #[test]
    fn method_dual_leaf_records_inv_vl_downgrade() {
        // Honest per-method posture: dual-leaf REMOVES INV-VL (issuer-honesty
        // downgrade, #769 accepted, CR-G8); string-canonical retains it.
        assert!(!CommitmentMethod::StringCanonicalV1.removes_inv_vl());
        assert!(CommitmentMethod::DualLeafV1.removes_inv_vl());
        // dual-leaf is still production-selectable (the #769-accepted method), even
        // though it carries the downgrade — `is_production_selectable` is NOT an
        // audit/soundness claim, only "not a research-only dial".
        assert!(CommitmentMethod::StringCanonicalV1.is_production_selectable());
        assert!(CommitmentMethod::DualLeafV1.is_production_selectable());
    }

    // value-only is the research dial: gated OFF by default. With the feature ON it
    // exists but is NEVER production-selectable and loses term identity (removes
    // INV-VL is trivially true — there is no lexical component at all). §10 default (2).
    #[cfg(feature = "commitment-value-only")]
    #[test]
    fn value_only_is_research_dial_only() {
        let m = CommitmentMethod::ValueOnlyV1;
        assert!(!m.is_production_selectable(), "value-only must never be production-selectable");
        assert!(m.removes_inv_vl());
        assert_eq!(m.scheme_iri(), "https://sparq.dev/ns/zk#poseidon2-valuehook-v1");
        assert_eq!(CommitmentMethod::from_scheme_iri(m.scheme_iri()), Some(m));
    }

    // With the feature OFF the value-only IRI must be UNKNOWN (rejected): a build
    // that did not opt into the research dial cannot select it.
    #[cfg(not(feature = "commitment-value-only"))]
    #[test]
    fn value_only_iri_is_rejected_without_the_feature() {
        assert_eq!(
            CommitmentMethod::from_scheme_iri("https://sparq.dev/ns/zk#poseidon2-valuehook-v1"),
            None,
            "value-only must be unselectable when the commitment-value-only feature is OFF"
        );
    }

    /// Every `CommitmentMethod` variant compiled into THIS build (the
    /// feature-state-aware set the round-trip/distinctness tests sweep).
    fn commitment_methods_in_build() -> Vec<CommitmentMethod> {
        #[allow(unused_mut)]
        let mut v = vec![CommitmentMethod::StringCanonicalV1, CommitmentMethod::DualLeafV1];
        #[cfg(feature = "commitment-value-only")]
        v.push(CommitmentMethod::ValueOnlyV1);
        v
    }

    #[test]
    fn leaf_index_resolves_canonical_triples() {
        let salt = salt_from_bytes(&[7u8; 32]);
        let c = commit_triples(&bnode_graph("x", "v"), salt).unwrap();
        for (i, t) in c.canonical.triples.iter().enumerate() {
            assert_eq!(c.leaf_index(t), Some(i));
        }
        let absent = Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/zzz").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            Term::Literal(Literal::new_simple_literal("v")),
        );
        assert_eq!(c.leaf_index(&absent), None);
    }

    // [OPUS-4.8] sq-qcnn.25: DETERMINISM + resolution value pins. Assert exact
    // VALUES (not just no-panic) over the commitment/leaf-resolution surface.
    // NO soundness claim (sq-qhy4).

    #[test]
    fn leaf_index_map_maps_every_canonical_triple_to_its_position() {
        // Direct test of `leaf_index_map`: the bulk map must agree exactly with the
        // per-triple `leaf_index`, be complete (one entry per canonical triple), and
        // omit absent triples. (Kills `leaf_index_map -> HashMap::new()`.)
        let salt = salt_from_bytes(&[7u8; 32]);
        let c = commit_triples(&bnode_graph("x", "v"), salt).unwrap();
        let map = c.leaf_index_map();
        assert_eq!(map.len(), c.canonical.triples.len());
        assert!(!map.is_empty());
        for (i, t) in c.canonical.triples.iter().enumerate() {
            assert_eq!(map.get(t), Some(&i), "map must resolve triple {} to leaf {}", i, i);
            assert_eq!(c.leaf_index(t), Some(i), "map must agree with leaf_index");
        }
        let absent = Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/absent").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            Term::Literal(Literal::new_simple_literal("v")),
        );
        assert!(!map.contains_key(&absent));
    }

    #[test]
    fn commit_error_display_is_exact() {
        // `From<CanonError>` wraps a canonicalization failure, and Display passes the
        // inner message through verbatim (covers the `From` + `Canon` Display arms;
        // kills `fmt -> Ok(Default::default())`).
        let inner = CanonError::Bridge("boom".to_string());
        let inner_msg = inner.to_string();
        let wrapped: CommitError = inner.into();
        assert!(matches!(wrapped, CommitError::Canon(_)));
        assert_eq!(wrapped.to_string(), inner_msg);
        // The `UncommittableTerm` Display is the exact "uncommittable term: {t}" form.
        let ut = CommitError::UncommittableTerm("http://ex/quoted".to_string());
        assert_eq!(ut.to_string(), "uncommittable term: http://ex/quoted");
    }

    #[test]
    fn commitment_pins_poseidon2_sponge_over_encoded_leaves() {
        // The whole-pipeline determinism contract: C(G) is EXACTLY the Poseidon2
        // sponge over the ordered leaves, and each leaf is EXACTLY `encode_triple`
        // of the canonical triple at that index. Recomputed independently from the
        // public primitives so a mutation of the leaf loop or the final sponge is
        // caught (not merely a "commit twice is equal" tautology).
        let salt = salt_from_bytes(&[7u8; 32]);
        let c = commit_triples(&bnode_graph("x", "v"), salt).unwrap();
        let recomputed_leaves: Vec<Fr> = c
            .canonical
            .triples
            .iter()
            .map(|t| crate::encode::encode_triple(t, &salt).unwrap())
            .collect();
        assert_eq!(c.leaves, recomputed_leaves, "leaves must be encode_triple per canonical triple");
        assert_eq!(
            c.commitment,
            poseidon2::hash(&recomputed_leaves),
            "commitment must be the Poseidon2 sponge over the leaf sequence"
        );
        // Byte-level determinism: same input -> same commitment bytes.
        let c2 = commit_triples(&bnode_graph("x", "v"), salt).unwrap();
        assert_eq!(
            crate::field::field_to_be_bytes_32(&c.commitment),
            crate::field::field_to_be_bytes_32(&c2.commitment),
        );
    }

    #[test]
    fn commit_graph_content_matches_commit_triples_over_materialized_triples() {
        use sparq_core::Graph;
        // `commit_graph_content` must equal `commit_triples` over the graph's
        // materialized triples under the same salt (covers the graph-content entry
        // point; asserts the two commit doors agree by construction).
        let salt = salt_from_bytes(&[5u8; 32]);
        let g = Graph::load_str(
            "<http://ex/s> <http://ex/p> \"o\" .\n<http://ex/s> <http://ex/q> <http://ex/o2> .",
            "turtle",
        )
        .unwrap();
        let from_store = commit_graph_content(&g, salt).unwrap();
        let triples = crate::canon::graph_triples(&g).unwrap();
        let from_triples = commit_triples(&triples, salt).unwrap();
        assert_eq!(from_store.commitment, from_triples.commitment);
        assert_eq!(from_store.leaves, from_triples.leaves);
        assert_eq!(from_store.canonical.lines, from_triples.canonical.lines);
    }
}
