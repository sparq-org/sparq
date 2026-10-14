//! The per-source **privacy descriptor** — the new seam input (design record §4.2).
//!
//! [`sparq_fedplan::SourceDescriptor`] describes a source's *content* (predicates, classes,
//! char-sets, authorities). This module adds the orthogonal **privacy** declaration the
//! later routing pass needs: which of a source's predicates it is willing to disclose in the
//! clear, the opaque attestation-key id its graph is signed under, and a reserved
//! participation/authorisation field (the deferred B7 hook).
//!
//! # Default-DENY (the load-bearing posture)
//!
//! The whole point of the seam is to *minimise* what leaves a source in the clear, so the
//! safe default is to treat **every** predicate as PRIVATE — disclosable only when the source
//! has explicitly opted it in. [`SourcePrivacyDescriptor::default`] therefore discloses
//! NOTHING, and [`SourcePrivacyDescriptor::may_disclose`] returns `false` for any predicate
//! that was not explicitly marked [`Disclosability::Public`]. A later pass *reads* the
//! descriptor, and the source re-checks it fail-closed against the declared envelope (the design
//! record's constraint C-B), so a plan whose envelope declares an over-disclosure is rejected,
//! not honoured. That re-check covers the declared envelope only.
//!
//! This module performs NO MPC and runs NO privacy-bearing computation — it is a typed policy
//! declaration. It makes no soundness/privacy claim; the privacy-bearing phases that consume
//! it are deferred and audit-gated (sq-9hrn / sq-qhy4). See the crate `README.md`.
//!
//! sq-2q1x.

use std::collections::BTreeMap;

use sparq_fedplan::SourceId;

/// Whether a source is willing to expose a predicate's bindings **in the clear** to the
/// federation, or insists they stay private (and therefore MPC-routed by a later phase).
///
/// The default for any predicate NOT named in a [`SourcePrivacyDescriptor`] is
/// [`Disclosability::Private`] — see the module-level default-deny note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Disclosability {
    /// The source explicitly permits this predicate's bindings to be revealed in the clear
    /// (e.g. a global-IRI membership fact — public by convention #6 of the design record).
    /// This is the ONLY value that lets [`SourcePrivacyDescriptor::may_disclose`] return
    /// `true`, and it must be set explicitly: there is no implicit disclosure.
    Public,
    /// The bindings must never leave the source in the clear (e.g. a salary). The default for
    /// every predicate. A later routing phase would route an operator over a private predicate
    /// `Hidden` (MPC), never `Disclosed`.
    Private,
}

/// A federated source's **privacy declaration** — the orthogonal companion to
/// [`sparq_fedplan::SourceDescriptor`]. Built with the typed [`SourcePrivacyDescriptor::builder`].
///
/// Posture: **default-DENY**. Only predicates explicitly marked [`Disclosability::Public`]
/// are disclosable; everything else is private. This is the source's OWN declaration; a
/// later (deferred) routing phase reads it, but the source re-enforces it fail-closed.
///
/// This type carries NO secret material and runs NO privacy-bearing logic — it is a typed
/// policy record. It makes no soundness/privacy claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePrivacyDescriptor {
    /// Which source this declaration is for (the same stable id `sparq-fedplan` keys on).
    id: SourceId,
    /// Per-predicate disclosability. A predicate ABSENT from this map is `Private` (default-
    /// deny) — only an explicit [`Disclosability::Public`] entry permits disclosure. Kept in a
    /// `BTreeMap` so iteration order is deterministic (the seam is deterministic throughout).
    disclosability: BTreeMap<String, Disclosability>,
    /// The OPAQUE attestation-key id the source's graph is signed under, if declared. Collected
    /// by a later phase into the disclosed key-set the (deferred, audit-gated) collaborative
    /// proof would bind membership to (convention #3). It is an opaque label at this tier — NO
    /// signature is verified and NO attestation is performed here.
    attestation_key_id: Option<String>,
    /// RESERVED participation/authorisation field (the deferred B7 hook — WAC/ACP-aware source
    /// skipping). `None` means "no authorisation policy declared at this tier". Its full design
    /// is out of scope for this skeleton; the field is reserved so the later phase has a place
    /// to read it without a breaking change.
    participates: Option<bool>,
}

impl SourcePrivacyDescriptor {
    /// Starts a typed builder for the source named `id`. The builder starts **default-deny**:
    /// no predicate is disclosable until [`SourcePrivacyDescriptorBuilder::public_predicate`]
    /// names it.
    pub fn builder(id: SourceId) -> SourcePrivacyDescriptorBuilder {
        SourcePrivacyDescriptorBuilder {
            id,
            disclosability: BTreeMap::new(),
            attestation_key_id: None,
            participates: None,
        }
    }

    /// A default-DENY descriptor for `id` that discloses NOTHING: every predicate is
    /// [`Disclosability::Private`] and no attestation key / authorisation is declared. This is
    /// the safe baseline — the most private possible declaration.
    pub fn deny_all(id: SourceId) -> SourcePrivacyDescriptor {
        SourcePrivacyDescriptor::builder(id).build()
    }

    /// The source this declaration is for.
    pub fn id(&self) -> &SourceId {
        &self.id
    }

    /// The declared disclosability of `predicate`. Returns [`Disclosability::Private`] for any
    /// predicate that was NOT explicitly marked public — the default-deny posture.
    pub fn disclosability(&self, predicate: &str) -> Disclosability {
        self.disclosability
            .get(predicate)
            .copied()
            .unwrap_or(Disclosability::Private)
    }

    /// Whether this source permits `predicate`'s bindings to be disclosed in the clear.
    ///
    /// **Default-deny invariant:** returns `true` ONLY when the source explicitly marked
    /// `predicate` as [`Disclosability::Public`]; `false` for every other predicate (including
    /// any never named in this descriptor). There is no implicit or transitive disclosure.
    pub fn may_disclose(&self, predicate: &str) -> bool {
        self.disclosability(predicate) == Disclosability::Public
    }

    /// The opaque attestation-key id the source's graph is signed under, if declared. Opaque at
    /// this tier — NO signature is verified here (the attestation half is deferred + audit-gated).
    pub fn attestation_key_id(&self) -> Option<&str> {
        self.attestation_key_id.as_deref()
    }

    /// The RESERVED participation/authorisation flag, if declared (the deferred B7 hook). `None`
    /// means no authorisation policy was declared at this tier.
    pub fn participates(&self) -> Option<bool> {
        self.participates
    }

    /// The predicates this source has explicitly marked disclosable (public), in deterministic
    /// ascending order. Every OTHER predicate is private by default.
    pub fn public_predicates(&self) -> impl Iterator<Item = &str> {
        self.disclosability
            .iter()
            .filter(|(_, d)| **d == Disclosability::Public)
            .map(|(p, _)| p.as_str())
    }
}

/// Builder for a [`SourcePrivacyDescriptor`]. Starts **default-deny** — call
/// [`SourcePrivacyDescriptorBuilder::public_predicate`] for each predicate the source is
/// willing to disclose in the clear; everything else stays private.
#[derive(Debug, Clone)]
pub struct SourcePrivacyDescriptorBuilder {
    id: SourceId,
    disclosability: BTreeMap<String, Disclosability>,
    attestation_key_id: Option<String>,
    participates: Option<bool>,
}

impl SourcePrivacyDescriptorBuilder {
    /// Explicitly marks `predicate` **disclosable in the clear** ([`Disclosability::Public`]).
    /// This is the ONLY way to opt a predicate out of the default-private posture.
    pub fn public_predicate(mut self, predicate: impl Into<String>) -> Self {
        self.disclosability
            .insert(predicate.into(), Disclosability::Public);
        self
    }

    /// Explicitly records `predicate` as [`Disclosability::Private`]. This is the default for
    /// any un-named predicate, so it is only needed to OVERRIDE a prior `public_predicate` call
    /// for the same predicate on this builder — it documents intent and makes the policy
    /// explicit. Provided for completeness; the absence of an entry is already private.
    pub fn private_predicate(mut self, predicate: impl Into<String>) -> Self {
        self.disclosability
            .insert(predicate.into(), Disclosability::Private);
        self
    }

    /// Declares the opaque attestation-key id the source's graph is signed under. Opaque label
    /// only — NO signature is verified here (the attestation half is deferred + audit-gated).
    pub fn attestation_key_id(mut self, key_id: impl Into<String>) -> Self {
        self.attestation_key_id = Some(key_id.into());
        self
    }

    /// Sets the RESERVED participation/authorisation flag (the deferred B7 hook). Left `None`
    /// by default. Its full WAC/ACP-aware design is out of scope for this skeleton.
    pub fn participates(mut self, participates: bool) -> Self {
        self.participates = Some(participates);
        self
    }

    /// Finalises the descriptor. Any predicate not explicitly marked public is private
    /// (default-deny).
    pub fn build(self) -> SourcePrivacyDescriptor {
        SourcePrivacyDescriptor {
            id: self.id,
            disclosability: self.disclosability,
            attestation_key_id: self.attestation_key_id,
            participates: self.participates,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid() -> SourceId {
        SourceId::new("http://holder.example/")
    }

    #[test]
    fn default_deny_discloses_nothing() {
        // A bare descriptor (no predicate marked) must disclose NOTHING — the load-bearing
        // default-deny invariant.
        let d = SourcePrivacyDescriptor::deny_all(sid());
        assert!(!d.may_disclose("http://ex/salary"));
        assert!(!d.may_disclose("http://ex/memberOf"));
        assert!(!d.may_disclose("http://ex/anything-at-all"));
        assert_eq!(d.disclosability("http://ex/x"), Disclosability::Private);
        assert_eq!(d.public_predicates().count(), 0);
        assert_eq!(d.attestation_key_id(), None);
        assert_eq!(d.participates(), None);
    }

    #[test]
    fn only_explicitly_public_predicates_disclose() {
        let d = SourcePrivacyDescriptor::builder(sid())
            .public_predicate("http://ex/memberOf")
            .attestation_key_id("did:key:z6Mk-issuer")
            .participates(true)
            .build();
        // Explicitly public ⇒ disclosable.
        assert!(d.may_disclose("http://ex/memberOf"));
        assert_eq!(
            d.disclosability("http://ex/memberOf"),
            Disclosability::Public
        );
        // Everything else stays private by default — even on a descriptor that DID mark
        // something public.
        assert!(!d.may_disclose("http://ex/salary"));
        assert_eq!(
            d.disclosability("http://ex/salary"),
            Disclosability::Private
        );
        // The opaque attestation id is recorded (NOT verified) and the reserved field round-trips.
        assert_eq!(d.attestation_key_id(), Some("did:key:z6Mk-issuer"));
        assert_eq!(d.participates(), Some(true));
        // Exactly one public predicate, deterministically reported.
        assert_eq!(
            d.public_predicates().collect::<Vec<_>>(),
            vec!["http://ex/memberOf"]
        );
    }

    #[test]
    fn private_predicate_overrides_a_prior_public_mark() {
        // The LAST mark on the builder wins; `private_predicate` can take back a `public_predicate`.
        let d = SourcePrivacyDescriptor::builder(sid())
            .public_predicate("http://ex/p")
            .private_predicate("http://ex/p")
            .build();
        assert!(!d.may_disclose("http://ex/p"));
        assert_eq!(d.public_predicates().count(), 0);
    }

    #[test]
    fn public_predicates_iterate_deterministically() {
        // Insertion order differs from sorted order; the BTreeMap-backed iterator is sorted, so
        // the seam stays deterministic.
        let d = SourcePrivacyDescriptor::builder(sid())
            .public_predicate("http://ex/c")
            .public_predicate("http://ex/a")
            .public_predicate("http://ex/b")
            .build();
        assert_eq!(
            d.public_predicates().collect::<Vec<_>>(),
            vec!["http://ex/a", "http://ex/b", "http://ex/c"]
        );
    }
}
