//! # `store.rs` — trust-document storage / authoring model (`store` feature, opt-in)
//!
//! Where trust rules LIVE, how they are versioned + revoked, and how a server-wide
//! default composes with per-`.acr` documents — the P4 storage/authoring model of the
//! design record (`research/solid-trust-graph-authz-design.md` §3.2 / §8 Q1; epic
//! `sq-pfae`, bead `sq-pfae.5`). `policy.rs` answers *"what is one admission rule"*;
//! this module answers *"where does the policy come from, which version is in force, and
//! how do a server default and a per-`.acr` document combine"*.
//!
//! ## Two layers, one composition — BOTH, with per-`.acr` NARROWING (design §3.2)
//!
//! The §8 Q1 maintainer decision is **both** scopes, with the monotone discipline
//! ACP inheritance already uses:
//!
//! - [`TrustLayer::ServerWide`] — the server-wide default trust policy (one per
//!   server). It establishes the **ceiling**: a `(source, statement-type)` pair the
//!   server default does NOT trust can never be admitted, no matter what a deeper
//!   document says.
//! - [`TrustLayer::Container`] — a per-`.acr` trust document anchored at a container /
//!   resource path. It may only **NARROW** the server default: tighten the freshness
//!   window, restrict the scope to a sub-tree, or drop a source — it can **NEVER**
//!   introduce a new trusted `(source, statement-type)` the server default did not
//!   already grant. Broadening fails closed (the rule is simply not in the effective
//!   set). This is the load-bearing soundness property of the storage model
//!   ([`TrustStore::effective_rules`]): a writer who controls one container's `.acr`
//!   cannot widen trust beyond the server administrator's ceiling.
//!
//! ## Control-gating, versioning, revocation
//!
//! - **Control-gated.** A [`TrustDocument`] can only be built behind a
//!   [`ControlGate`](crate::policy::ControlGate) — the SAME fail-closed channel as
//!   `.acl`/`.acr` (§3.2). A document that did not arrive through Control cannot enter
//!   the store; there is no un-gated constructor.
//! - **Versioned.** Each document carries a monotone [`TrustDocument::version`]. An
//!   author bumps it on every edit; the store rejects a stale (≤ current) version on
//!   update ([`TrustStore::put`] returns [`StoreError::StaleVersion`]) so a replayed
//!   old document can never silently roll trust back.
//! - **Revoked.** Revocation is **document removal** ([`TrustStore::revoke`]): the
//!   document leaves the store and the [`PolicyVersion`] for every affected target
//!   bumps, so the composed admission decision is invalidated. (This matches the
//!   epoch-bump-on-rematerialise discipline — revocation propagates by re-materialise,
//!   §3.3 / G5; there is no in-place retraction inside a single reasoner run.)
//!
//! ## The admission cache key (composes with the sparq-solid epoch cache)
//!
//! [`AdmissionCacheKey`] `= (evidence_hash, policy_version)` is the design's §9.1
//! cache-key recipe made concrete:
//!
//! - `evidence_hash` — a stable hash of the presented credential's RDFC-1.0 commitment
//!   + bound holder (the per-credential evidence). Two requests presenting the SAME
//!   credential for the SAME holder share it.
//! - `policy_version` — the [`PolicyVersion`] for the target: the aggregate of the
//!   server-wide document version and every per-`.acr` document in force on the path to
//!   the target. ANY contributing document edit / revocation bumps it.
//!
//! It composes with the sparq-solid **epoch cache**: the host caches an admission
//! verdict under `(epoch, AdmissionCacheKey)`. A re-materialise bumps the epoch (drops
//! everything); a trust-document edit/revoke bumps the `policy_version` (drops only the
//! entries for affected targets); a different credential changes the `evidence_hash`.
//! So a stale verdict is NEVER served after either the auth view OR the trust policy
//! changed — the load-bearing invalidation property this module exists to provide.
//!
//! ## Honest scope
//!
//! This is the STORAGE / AUTHORING model, not a new admission primitive: it decides
//! *which* [`TrustRule`]s are in force for a target and a cache key over them. It adds
//! NO cryptographic guarantee; the admission gate ([`mod@crate::admit`]) and its honest
//! caveats (no privacy/unlinkability; operator-asserted issuer keys; externally
//! UNAUDITED ZK estate — `sq-qhy4`) are unchanged.
//!
//! ## Certification-edge composition (`cert-graph` feature, opt-in)
//!
//! When the `cert-graph` feature is enabled, a [`TrustDocument`] may carry signed
//! [`crate::graph::Certification`] edges (via [`TrustDocument::with_certifications`]).
//! The store's [`TrustStore::effective_rules`] runs
//! `crate::graph::derive_effective_rules` over the document's direct rules and its
//! certification edges, so a certified issuer's rules are composed through the SAME
//! server-ceiling + per-`.acr` narrowing discipline — derived rules can only NARROW,
//! never BROADEN, the certifier's own anchor.
//!
//! The cache-safety invariant is load-bearing: [`TrustStore::policy_version`] folds
//! the certifier IRI, certified-issuer IRI, and the validity window (`valid_from` +
//! `valid_until`) of every certification into the hash.  Revoking a certification OR
//! re-authoring its validity window changes at least one of those fields (the document's
//! certification set loses an edge or the window shifts), which changes `policy_version`
//! and therefore the [`AdmissionCacheKey`].  Wall-clock expiry of an UNCHANGED
//! certification (time passing a fixed `valid_until` with no document edit) does NOT
//! turn the cache key over on its own — it propagates by re-materialise / epoch-bump
//! (like revocation), bounded by the host epoch cadence; the residual stale-authority
//! window is the acknowledged open problem tracked by `sq-l5og`.
//!
//! **Zero certifications ⇒ byte-identical to the pre-cert path.** With no
//! certifications the `with_certifications` constructor is unused, the `derive_effective_rules`
//! call is a no-op that returns `direct_rules` verbatim, and the `policy_version`
//! hash is unchanged (no certification entries are folded in).
//!
//! sq-pfae.5 P4 storage/authoring model (issue #940). Written while Fable
//! unavailable — flag for re-review when Fable returns. 🤖 SPARQ agent — trust-graph
//! authorisation PoC.
//! sq-pfae.16 — cert-graph composition into TrustStore. 🤖 SPARQ agent.

use crate::admit::{scope_covers, PresentedCredential};
use crate::policy::{ControlGate, TrustRule};
use oxrdf::NamedNode;
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::field_to_be_bytes_32;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

/// Where a trust document lives (§3.2 / §8 Q1 — the storage layer). The model is
/// **both** server-wide and per-`.acr`, with [`TrustLayer::Container`] narrowing the
/// [`TrustLayer::ServerWide`] default.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TrustLayer {
    /// The server-wide default trust policy (one per server). It establishes the
    /// ceiling on what any deeper document may admit.
    ServerWide,
    /// A per-`.acr` trust document anchored at a container / resource path (the IRI of
    /// the container or resource the `.acr` governs). It may only narrow the ceiling.
    Container(NamedNode),
}

impl TrustLayer {
    /// The anchor IRI of a [`TrustLayer::Container`], or `None` for the server-wide
    /// layer. Used to order layers by path-specificity (a deeper anchor is more
    /// specific).
    pub fn anchor(&self) -> Option<&NamedNode> {
        match self {
            TrustLayer::ServerWide => None,
            TrustLayer::Container(a) => Some(a),
        }
    }

    /// Whether this layer's anchor covers `target` (so the layer's rules are CANDIDATES
    /// for the target). The server-wide layer covers every target; a container layer
    /// covers a target iff the target is the anchor or a descendant of it (Solid
    /// slash-semantics, the same containment [`scope_covers`] uses).
    pub fn covers(&self, target: &NamedNode) -> bool {
        match self {
            TrustLayer::ServerWide => true,
            TrustLayer::Container(anchor) => scope_covers(anchor, target),
        }
    }
}

/// A Control-gated, versioned trust document at a [`TrustLayer`]. Holds the
/// already-parsed [`TrustRule`]s (parse them with [`crate::parse_policy`], which itself
/// requires a [`ControlGate`]).
///
/// A document can only be built with [`TrustDocument::new`], which REQUIRES a
/// [`ControlGate`]: a document that did not arrive through the trusted `.acl`/`.acr`
/// channel cannot enter the store (fail-closed, §3.2).
///
/// When the `cert-graph` feature is enabled, the document may additionally carry signed
/// certification edges ([`TrustDocument::with_certifications`]) that the store feeds into
/// `crate::graph::derive_effective_rules` to derive additional attenuated rules ahead of
/// the admission gate.
#[derive(Debug, Clone)]
pub struct TrustDocument {
    /// Where the document lives (server-wide vs a per-`.acr` container).
    layer: TrustLayer,
    /// The monotone version the author stamps. Bumped on every edit; a stale version is
    /// rejected on update ([`StoreError::StaleVersion`]).
    version: u64,
    /// The parsed admission rules this document carries.
    rules: Vec<TrustRule>,
    /// Signed certification edges attached to this document (OPT-IN `cert-graph` feature
    /// only). Empty when the feature is off or the document was built with `new`.
    #[cfg(feature = "cert-graph")]
    certifications: Vec<crate::graph::Certification>,
}

impl TrustDocument {
    /// Build a Control-gated trust document. Possession of a [`ControlGate`] asserts the
    /// document arrived through the `.acl`/`.acr` channel (§3.2) — there is no un-gated
    /// constructor, so an untrusted writer's document can never enter the store.
    pub fn new(
        layer: TrustLayer,
        version: u64,
        rules: Vec<TrustRule>,
        _gate: ControlGate,
    ) -> TrustDocument {
        TrustDocument {
            layer,
            version,
            rules,
            #[cfg(feature = "cert-graph")]
            certifications: Vec::new(),
        }
    }

    /// Build a Control-gated trust document that also carries signed certification edges.
    /// The certifications are fed into `crate::graph::derive_effective_rules` by the store's
    /// `effective_rules` method, using `now_unix_secs` at call time (depth-1 closure).
    ///
    /// Require: `cert-graph` feature (and `framework-vocab`, which it enables).
    ///
    /// The `_gate` token asserts the document — including its certifications — arrived
    /// through the trusted `.acl`/`.acr` channel (§3.2). There is no un-gated constructor.
    ///
    /// ## Cache-safety (load-bearing)
    ///
    /// The certifier IRI, certified-issuer IRI, and validity window (`valid_from` +
    /// `valid_until`) of every certification are folded into
    /// `TrustStore::policy_version`.  Revoking a certification OR re-authoring its
    /// validity window changes `policy_version` and therefore the `AdmissionCacheKey`.
    /// Wall-clock expiry of an UNCHANGED cert (time passing a fixed `valid_until` with
    /// no document edit) propagates by re-materialise / epoch-bump (like revocation),
    /// bounded by the host epoch cadence — NOT by the cache key alone; the residual
    /// stale-authority window is tracked by `sq-l5og`.
    #[cfg(feature = "cert-graph")]
    #[cfg_attr(docsrs, doc(cfg(feature = "cert-graph")))]
    pub fn with_certifications(
        layer: TrustLayer,
        version: u64,
        rules: Vec<TrustRule>,
        certifications: Vec<crate::graph::Certification>,
        _gate: ControlGate,
    ) -> TrustDocument {
        TrustDocument {
            layer,
            version,
            rules,
            certifications,
        }
    }

    /// The layer this document lives at.
    pub fn layer(&self) -> &TrustLayer {
        &self.layer
    }

    /// The monotone document version.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The admission rules this document carries.
    pub fn rules(&self) -> &[TrustRule] {
        &self.rules
    }

    /// The signed certification edges attached to this document (`cert-graph` feature only).
    /// Empty when the feature is off or the document was built with `new` (no certifications).
    #[cfg(feature = "cert-graph")]
    #[cfg_attr(docsrs, doc(cfg(feature = "cert-graph")))]
    pub fn certifications(&self) -> &[crate::graph::Certification] {
        &self.certifications
    }
}

/// The aggregate policy version in force for a target: the combination of the
/// server-wide document version and every per-`.acr` document on the path to the
/// target. Any contributing document edit / revocation changes it, so it is a stable
/// cache-invalidation token. Composes with the sparq-solid epoch cache (§9.1).
///
/// It is an opaque 64-bit token — callers compare it for equality / use it as a cache
/// key, never interpret its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PolicyVersion(pub u64);

/// A composed admission cache key: `(evidence_hash, policy_version)` (design §9.1). Two
/// admission requests share a key iff they present the SAME credential evidence for the
/// SAME holder AND the trust policy in force for the target is at the SAME version. It
/// composes with the sparq-solid epoch cache: cache an admission verdict under
/// `(epoch, AdmissionCacheKey)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AdmissionCacheKey {
    /// A stable hash of the presented credential's RDFC-1.0 commitment + bound holder
    /// (the per-credential evidence). `0` is reserved for a credential whose graph
    /// could not be committed (fail-closed: such a credential admits nothing anyway, so
    /// the key never serves a real verdict).
    pub evidence_hash: u64,
    /// The [`PolicyVersion`] for the target.
    pub policy_version: PolicyVersion,
}

/// Why a trust-store update was rejected (fail-closed: a rejected update leaves the
/// store unchanged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// A [`TrustStore::put`] presented a version `≤` the version already stored at that
    /// layer — a stale or replayed document. The existing document is kept.
    StaleVersion {
        /// The version already in the store.
        current: u64,
        /// The (rejected) version that was presented.
        presented: u64,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::StaleVersion { current, presented } => write!(
                f,
                "stale trust-document version {} (store is at {}): rejected fail-closed",
                presented, current
            ),
        }
    }
}

impl std::error::Error for StoreError {}

/// The trust-document store: a server-wide default plus zero-or-more per-`.acr`
/// documents, composed with per-`.acr` NARROWING (§3.2). It owns the
/// version/revocation bookkeeping and produces the [`PolicyVersion`] +
/// [`AdmissionCacheKey`] that compose with the sparq-solid epoch cache.
#[derive(Debug, Clone, Default)]
pub struct TrustStore {
    /// The server-wide default (`None` until one is installed — fail-closed: with no
    /// server default, NOTHING is admissible, because a per-`.acr` document can only
    /// narrow a ceiling that does not exist).
    server_wide: Option<TrustDocument>,
    /// The per-`.acr` documents, keyed by their anchor IRI. `BTreeMap` for a
    /// deterministic, anchor-ordered iteration (so the composed result is stable).
    containers: BTreeMap<String, TrustDocument>,
}

impl TrustStore {
    /// An empty store (no server default, no per-`.acr` documents). Fail-closed:
    /// admits nothing until a server-wide default is installed.
    pub fn new() -> TrustStore {
        TrustStore::default()
    }

    /// Install or update a trust document. The version MUST be strictly greater than the
    /// version currently stored at the same layer — a stale/replayed document is rejected
    /// with [`StoreError::StaleVersion`] and the store is left unchanged (monotone
    /// versioning, §3.2). A brand-new layer accepts any version.
    pub fn put(&mut self, doc: TrustDocument) -> Result<(), StoreError> {
        match doc.layer.clone() {
            TrustLayer::ServerWide => {
                if let Some(existing) = &self.server_wide {
                    if doc.version <= existing.version {
                        return Err(StoreError::StaleVersion {
                            current: existing.version,
                            presented: doc.version,
                        });
                    }
                }
                self.server_wide = Some(doc);
            }
            TrustLayer::Container(anchor) => {
                let key = anchor.as_str().to_string();
                if let Some(existing) = self.containers.get(&key) {
                    if doc.version <= existing.version {
                        return Err(StoreError::StaleVersion {
                            current: existing.version,
                            presented: doc.version,
                        });
                    }
                }
                self.containers.insert(key, doc);
            }
        }
        Ok(())
    }

    /// Revoke (remove) the document at a layer. Returns `true` if a document was present
    /// and removed. Revocation is document removal: after it, [`TrustStore::policy_version`]
    /// for every affected target changes, invalidating any cached admission verdict
    /// (§3.3 / G5 — revocation propagates by re-materialise, not in-place retraction).
    pub fn revoke(&mut self, layer: &TrustLayer) -> bool {
        match layer {
            TrustLayer::ServerWide => self.server_wide.take().is_some(),
            TrustLayer::Container(anchor) => self.containers.remove(anchor.as_str()).is_some(),
        }
    }

    /// The effective trust rules in force for `target`: the server-wide default,
    /// NARROWED by every per-`.acr` document on the path to the target.
    ///
    /// The composition is **monotone narrowing** (§3.2): a per-`.acr` rule enters the
    /// effective set ONLY if it is covered by (a narrowing of) some server-default rule
    /// for the SAME `(source, statement-type-shape)` — i.e. the per-`.acr` document may
    /// restrict scope / freshness, but may NOT trust a `(source, type)` the server
    /// default did not. A per-`.acr` rule with no matching server-default rule is
    /// DROPPED (broadening is fail-closed).
    ///
    /// Concretely, the effective rule for a `(source, shape-key)` the server trusts is
    /// the **tightest** of the server default and any covering per-`.acr` narrowing:
    /// the smaller freshness window and the more-specific scope. A target with no
    /// server default admits nothing (empty result).
    ///
    /// When the `cert-graph` feature is enabled and the server-wide document carries
    /// certification edges, `crate::graph::derive_effective_rules` is run over the
    /// direct rules BEFORE the per-`.acr` narrowing step, so certified issuers are
    /// admitted through the SAME ceiling + narrowing discipline as direct rules.
    /// `now_unix_secs` is the evaluation instant for certification windows; pass the
    /// current Unix second count. An expired or not-yet-valid certification contributes
    /// NOTHING (fail-closed on time, identical to the standalone `graph` path).
    pub fn effective_rules(&self, target: &NamedNode) -> Vec<TrustRule> {
        self.effective_rules_at(target, now_unix_secs())
    }

    /// Like [`effective_rules`](Self::effective_rules) but with an explicit evaluation
    /// instant (`now_unix_secs`) for certification windows. The public `effective_rules`
    /// method calls this with the system clock; tests call this directly with a fixed
    /// instant to make certification expiry deterministic.
    pub fn effective_rules_at(&self, target: &NamedNode, now_secs: i64) -> Vec<TrustRule> {
        let Some(server) = &self.server_wide else {
            return Vec::new(); // fail-closed: no ceiling ⇒ nothing admissible
        };

        // Step 1 (cert-graph feature): derive additional attenuated rules from signed
        // certification edges attached to the server-wide document, BEFORE per-.acr
        // narrowing. The derived rules satisfy derived ⊆ (anchor ∩ cert scope ∩
        // validity window) — they can only NARROW, never widen, the certifier's
        // authority. Zero certifications ⇒ direct_rules unchanged (strict additivity).
        #[cfg(feature = "cert-graph")]
        let direct_rules: Vec<TrustRule> = crate::graph::derive_effective_rules(
            server.rules(),
            server.certifications(),
            now_secs,
            1, // v1: depth-1 closure (sq-tu4e discipline)
        );
        #[cfg(not(feature = "cert-graph"))]
        let direct_rules: Vec<TrustRule> = server.rules().to_vec();
        // Suppress the unused-variable warning when cert-graph is off.
        let _ = now_secs;

        // The per-`.acr` documents whose anchor covers the target, from least specific
        // (shallowest container) to most specific (deepest). A deeper document narrows
        // a shallower one — but ALL are still bounded by the server ceiling.
        let mut covering: Vec<&TrustDocument> = self
            .containers
            .values()
            .filter(|d| d.layer.covers(target))
            .collect();
        covering.sort_by_key(|d| d.layer.anchor().map(|a| a.as_str().len()).unwrap_or(0));

        let mut out: Vec<TrustRule> = Vec::new();
        // Start from the direct rules (which already incorporate any certified issuers):
        // every rule that itself covers the target is a candidate for further narrowing.
        for drule in &direct_rules {
            if !scope_covers(&drule.scope, target) {
                continue;
            }
            // Narrow this direct rule by every covering per-`.acr` rule that matches its
            // (source, statement-type) AND is itself a narrowing (covered scope). A
            // per-`.acr` rule that does NOT match any direct rule is broadening — never
            // applied here, so it is dropped (fail-closed).
            let mut eff = drule.clone();
            for doc in &covering {
                for crule in doc.rules() {
                    if narrows(drule, crule, target) {
                        eff = tighten(&eff, crule);
                    }
                }
            }
            out.push(eff);
        }
        out
    }

    /// The [`PolicyVersion`] in force for `target`: a stable aggregate of the
    /// server-wide document version and every per-`.acr` document covering the target.
    /// Any contributing document edit (version bump) or revocation (removal) changes it,
    /// so it is a sound cache-invalidation token.
    ///
    /// When the `cert-graph` feature is enabled, the certifier IRI, certified-issuer IRI,
    /// and validity window (`valid_from` + `valid_until`) of every certification carried
    /// by the server-wide document are also folded in.  Revoking a certification (removing
    /// it from the set) OR re-authoring its validity window changes the hash and therefore
    /// the `AdmissionCacheKey` — no stale admission verdict survives a certification
    /// revocation or window re-authoring (the load-bearing cache-safety invariant for
    /// `sq-pfae.16`).  Wall-clock expiry of an UNCHANGED cert propagates by
    /// re-materialise / epoch-bump (like revocation), bounded by the host epoch cadence —
    /// NOT by the cache key alone; the residual stale-authority window is tracked by
    /// `sq-l5og`.
    ///
    /// **Zero certifications ⇒ byte-identical to the pre-cert path.** No certification
    /// entries are folded into the hasher, so the output is unchanged compared to a
    /// document built with `new`.
    pub fn policy_version(&self, target: &NamedNode) -> PolicyVersion {
        let mut hasher = DefaultHasher::new();
        // Domain tag so a server-wide-version `N` never collides with a container
        // version `N`.
        "trust-policy-version:v1".hash(&mut hasher);
        match &self.server_wide {
            Some(d) => {
                0u8.hash(&mut hasher); // present
                d.version.hash(&mut hasher);
                // cert-graph: fold the certification set identity so revocation/expiry
                // changes policy_version and therefore AdmissionCacheKey (cache-safety).
                // We fold certifier IRI + certified-issuer IRI + valid_from + valid_until
                // for each edge. The ORDER matters for stability: certifications are folded
                // in their stored order (the caller is responsible for a stable ordering —
                // the store does not re-sort them, matching the `graph.rs` fast path).
                #[cfg(feature = "cert-graph")]
                for cert in d.certifications() {
                    "cert:".hash(&mut hasher);
                    cert.certifier.as_str().hash(&mut hasher);
                    cert.certified_issuer.as_str().hash(&mut hasher);
                    cert.valid_from_unix_secs.hash(&mut hasher);
                    cert.valid_until_unix_secs.hash(&mut hasher);
                }
            }
            None => 1u8.hash(&mut hasher), // absent — still a distinct, stable state
        }
        // Containers covering the target, in deterministic anchor order (BTreeMap is
        // already sorted by the String key). Both the anchor IRI AND the version are
        // folded in, so adding/removing a covering document changes the result.
        for (anchor, doc) in &self.containers {
            if doc.layer.covers(target) {
                anchor.hash(&mut hasher);
                doc.version.hash(&mut hasher);
            }
        }
        PolicyVersion(hasher.finish())
    }

    /// The composed [`AdmissionCacheKey`] for presenting `cred` (bound to `holder`) at
    /// `target`: `(evidence_hash, policy_version)`. Composes with the sparq-solid epoch
    /// cache — see the module docs.
    pub fn admission_cache_key(
        &self,
        cred: &PresentedCredential,
        holder: &NamedNode,
        target: &NamedNode,
    ) -> AdmissionCacheKey {
        AdmissionCacheKey {
            evidence_hash: evidence_hash(cred, holder),
            policy_version: self.policy_version(target),
        }
    }
}

/// Current Unix timestamp in seconds (used as the default `now_secs` for certification
/// window checks). Falls back to `0` on platforms where `SystemTime` is unavailable
/// (fail-closed: a window `[0, 0]` is trivially expired for any realistic cert).
fn now_unix_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A stable per-credential evidence hash: the RDFC-1.0 commitment over the credential
/// graph (the same canonical unit the admission gate and the ZK estate commit over)
/// folded with the bound holder WebID. `0` for a credential whose graph cannot be
/// committed (fail-closed — such a credential admits nothing).
fn evidence_hash(cred: &PresentedCredential, holder: &NamedNode) -> u64 {
    let salt = salt_from_bytes(&cred.salt);
    let Ok(commitment) = commit_triples(&cred.graph, salt) else {
        return 0; // uncommittable ⇒ reserved sentinel; never serves a real verdict
    };
    let mut hasher = DefaultHasher::new();
    "trust-evidence:v1".hash(&mut hasher);
    field_to_be_bytes_32(&commitment.commitment).hash(&mut hasher);
    holder.as_str().hash(&mut hasher);
    // The signature binds the credential to its issuer; fold it so two graphs with the
    // same commitment but different issuer signatures never share an evidence key.
    cred.issuer_signature_hex.hash(&mut hasher);
    hasher.finish()
}

/// Whether a per-`.acr` rule `crule` is a NARROWING of a server-default rule `srule`
/// for the target — i.e. it trusts the SAME source for the SAME statement-type-shape
/// and its scope is covered by (within) the server rule's scope. A per-`.acr` rule that
/// trusts a different source / type, or whose scope is NOT within the server rule's, is
/// NOT a narrowing (it would broaden) and is rejected.
fn narrows(srule: &TrustRule, crule: &TrustRule, target: &NamedNode) -> bool {
    // Same trusted source.
    if srule.source.as_str() != crule.source.as_str() {
        return false;
    }
    // Same statement-type: the issuer key the source signs with AND the shape must
    // match (a per-`.acr` document cannot swap the key or widen the shape).
    if srule.issuer_key != crule.issuer_key {
        return false;
    }
    if shape_key(srule) != shape_key(crule) {
        return false;
    }
    // The per-`.acr` rule must itself cover the target (it is in force here) AND its
    // scope must be WITHIN the server rule's scope (narrowing, never broadening).
    scope_covers(&crule.scope, target) && scope_covers(&srule.scope, &crule.scope)
}

/// Tighten an effective rule by a narrowing per-`.acr` rule: take the smaller freshness
/// window and the more-specific (descendant) scope. Never widens either dimension.
fn tighten(eff: &TrustRule, crule: &TrustRule) -> TrustRule {
    let fresh_within_secs = eff.fresh_within_secs.min(crule.fresh_within_secs);
    // The more-specific scope is the one the OTHER covers (a descendant string is
    // longer and is covered by its ancestor). `narrows` already proved
    // `scope_covers(eff.scope, crule.scope)`, so `crule.scope` is the descendant.
    let scope = crule.scope.clone();
    TrustRule {
        source: eff.source.clone(),
        issuer_key: eff.issuer_key,
        shape: eff.shape.clone(),
        scope,
        fresh_within_secs,
    }
}

/// A coarse, stable identity for a rule's statement-type shape: the sorted set of the
/// shape's `sh:targetSubjectsOf` predicate IRIs. Two rules with the same target
/// predicates are "the same statement-type" for the narrowing check. (The PoC's shapes
/// are single-predicate desugarings or `targetSubjectsOf`-anchored node-shapes, so this
/// is exact for the supported fragment; an inline property-shape difference that does
/// not change the target predicates is treated as the same type — a conservative,
/// fail-closed-leaning choice for the narrowing direction.)
fn shape_key(rule: &TrustRule) -> Vec<String> {
    use crate::vocab::SH_TARGET_SUBJECTS_OF;
    let mut preds: Vec<String> = rule
        .shape
        .triples
        .iter()
        .filter(|t| t.predicate.as_str() == SH_TARGET_SUBJECTS_OF)
        .filter_map(|t| match &t.object {
            oxrdf::Term::NamedNode(n) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect();
    preds.sort();
    preds.dedup();
    preds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{parse_policy, ControlGate};
    use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
    use sparq_zk::sig::SecretKey;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    /// Build a one-rule Control-gated policy graph (source / issuerKey / forPredicate /
    /// scope / freshWithin) and parse it into `Vec<TrustRule>`.
    fn policy_rules(
        rule_iri: &str,
        source: &str,
        key_hex: &str,
        predicate: &str,
        scope: &str,
        fresh: &str,
    ) -> Vec<TrustRule> {
        use crate::vocab::{
            FOR_PREDICATE, FRESH_WITHIN, ISSUER_KEY, RDF_TYPE, SCOPE, SOURCE, TRUST_RULE,
        };
        let s = NamedOrBlankNode::NamedNode(iri(rule_iri));
        let xsd_dur = iri("http://www.w3.org/2001/XMLSchema#duration");
        let triples = vec![
            Triple::new(s.clone(), iri(RDF_TYPE), Term::NamedNode(iri(TRUST_RULE))),
            Triple::new(s.clone(), iri(SOURCE), Term::NamedNode(iri(source))),
            Triple::new(
                s.clone(),
                iri(ISSUER_KEY),
                Term::Literal(Literal::new_simple_literal(key_hex)),
            ),
            Triple::new(
                s.clone(),
                iri(FOR_PREDICATE),
                Term::NamedNode(iri(predicate)),
            ),
            Triple::new(s.clone(), iri(SCOPE), Term::NamedNode(iri(scope))),
            Triple::new(
                s,
                iri(FRESH_WITHIN),
                Term::Literal(Literal::new_typed_literal(fresh, xsd_dur)),
            ),
        ];
        parse_policy(&triples, ControlGate::assert_control_gated()).unwrap()
    }

    /// A deterministic verifying-key hex (the value is irrelevant to the storage tests —
    /// they never verify a signature — but it must PARSE as a key).
    fn a_key_hex() -> String {
        let sk = SecretKey::from_seed(0xA11CE);
        sparq_zk::sig::public_key_to_hex(&sk.public_key())
    }

    fn gate() -> ControlGate {
        ControlGate::assert_control_gated()
    }

    #[test]
    fn empty_store_admits_nothing_fail_closed() {
        let store = TrustStore::new();
        let eff = store.effective_rules(&iri("https://pod.ex/docs/x"));
        assert!(eff.is_empty(), "no server default ⇒ nothing admissible");
    }

    #[test]
    fn server_wide_default_is_the_ceiling() {
        let key = a_key_hex();
        let server = policy_rules(
            "https://pod.ex/trust#r",
            "https://gov.example/issuer",
            &key,
            "http://schema.org/age",
            "https://pod.ex/", // server-wide container scope
            "P30D",
        );
        let mut store = TrustStore::new();
        store
            .put(TrustDocument::new(TrustLayer::ServerWide, 1, server, gate()))
            .unwrap();
        let eff = store.effective_rules(&iri("https://pod.ex/docs/x"));
        assert_eq!(eff.len(), 1, "the server rule covers the target");
        assert_eq!(eff[0].fresh_within_secs, 30 * 86_400);
    }

    #[test]
    fn per_acr_narrows_freshness_and_scope_never_broadens() {
        let key = a_key_hex();
        let server = policy_rules(
            "https://pod.ex/trust#r",
            "https://gov.example/issuer",
            &key,
            "http://schema.org/age",
            "https://pod.ex/",
            "P30D",
        );
        // Per-.acr on /docs/ narrows to P7D and scope /docs/.
        let acr = policy_rules(
            "https://pod.ex/docs/.acr#r",
            "https://gov.example/issuer",
            &key,
            "http://schema.org/age",
            "https://pod.ex/docs/",
            "P7D",
        );
        let mut store = TrustStore::new();
        store
            .put(TrustDocument::new(TrustLayer::ServerWide, 1, server, gate()))
            .unwrap();
        store
            .put(TrustDocument::new(
                TrustLayer::Container(iri("https://pod.ex/docs/")),
                1,
                acr,
                gate(),
            ))
            .unwrap();

        // Inside /docs/: the tighter P7D + the more-specific /docs/ scope win.
        let eff = store.effective_rules(&iri("https://pod.ex/docs/x"));
        assert_eq!(eff.len(), 1);
        assert_eq!(eff[0].fresh_within_secs, 7 * 86_400, "narrowed to P7D");
        assert_eq!(eff[0].scope.as_str(), "https://pod.ex/docs/");

        // Outside /docs/ (e.g. /photos/x): the per-.acr does NOT cover it, so the
        // server ceiling P30D applies unchanged.
        let eff_out = store.effective_rules(&iri("https://pod.ex/photos/x"));
        assert_eq!(eff_out.len(), 1);
        assert_eq!(eff_out[0].fresh_within_secs, 30 * 86_400);
    }

    #[test]
    fn per_acr_cannot_broaden_a_new_source_is_dropped() {
        let key = a_key_hex();
        // Server trusts ONLY gov for age.
        let server = policy_rules(
            "https://pod.ex/trust#r",
            "https://gov.example/issuer",
            &key,
            "http://schema.org/age",
            "https://pod.ex/",
            "P30D",
        );
        // A per-.acr tries to ALSO trust a different issuer for age — broadening.
        let rogue = policy_rules(
            "https://pod.ex/docs/.acr#r",
            "https://rogue.example/issuer",
            &key,
            "http://schema.org/age",
            "https://pod.ex/docs/",
            "P30D",
        );
        let mut store = TrustStore::new();
        store
            .put(TrustDocument::new(TrustLayer::ServerWide, 1, server, gate()))
            .unwrap();
        store
            .put(TrustDocument::new(
                TrustLayer::Container(iri("https://pod.ex/docs/")),
                1,
                rogue,
                gate(),
            ))
            .unwrap();
        let eff = store.effective_rules(&iri("https://pod.ex/docs/x"));
        // The rogue source is NOT in the effective set — only the server-trusted gov.
        assert_eq!(eff.len(), 1, "broadening per-.acr source is dropped");
        assert_eq!(eff[0].source.as_str(), "https://gov.example/issuer");
    }

    #[test]
    fn stale_version_is_rejected() {
        let key = a_key_hex();
        let mk = |v: &str| {
            policy_rules(
                "https://pod.ex/trust#r",
                "https://gov.example/issuer",
                &key,
                "http://schema.org/age",
                "https://pod.ex/",
                v,
            )
        };
        let mut store = TrustStore::new();
        store
            .put(TrustDocument::new(
                TrustLayer::ServerWide,
                5,
                mk("P30D"),
                gate(),
            ))
            .unwrap();
        // A replayed v3 (≤ 5) is rejected; the store is unchanged.
        let err = store
            .put(TrustDocument::new(
                TrustLayer::ServerWide,
                3,
                mk("P7D"),
                gate(),
            ))
            .unwrap_err();
        assert_eq!(
            err,
            StoreError::StaleVersion {
                current: 5,
                presented: 3
            }
        );
        // Equal version is also rejected (strictly-greater required).
        assert!(store
            .put(TrustDocument::new(
                TrustLayer::ServerWide,
                5,
                mk("P7D"),
                gate()
            ))
            .is_err());
        // A newer version is accepted.
        store
            .put(TrustDocument::new(
                TrustLayer::ServerWide,
                6,
                mk("P7D"),
                gate(),
            ))
            .unwrap();
    }

    #[test]
    fn policy_version_bumps_on_edit_and_revoke() {
        let key = a_key_hex();
        let mk = |scope: &str| {
            policy_rules(
                "https://pod.ex/trust#r",
                "https://gov.example/issuer",
                &key,
                "http://schema.org/age",
                scope,
                "P30D",
            )
        };
        let target = iri("https://pod.ex/docs/x");
        let mut store = TrustStore::new();
        store
            .put(TrustDocument::new(
                TrustLayer::ServerWide,
                1,
                mk("https://pod.ex/"),
                gate(),
            ))
            .unwrap();
        let v1 = store.policy_version(&target);

        // Adding a covering per-.acr document changes the version.
        store
            .put(TrustDocument::new(
                TrustLayer::Container(iri("https://pod.ex/docs/")),
                1,
                mk("https://pod.ex/docs/"),
                gate(),
            ))
            .unwrap();
        let v2 = store.policy_version(&target);
        assert_ne!(v1, v2, "adding a covering document bumps the policy version");

        // Editing it (version bump) changes the version again.
        store
            .put(TrustDocument::new(
                TrustLayer::Container(iri("https://pod.ex/docs/")),
                2,
                mk("https://pod.ex/docs/"),
                gate(),
            ))
            .unwrap();
        let v3 = store.policy_version(&target);
        assert_ne!(v2, v3, "editing a document bumps the policy version");

        // Revoking it changes the version again (back toward the server-only state).
        assert!(store.revoke(&TrustLayer::Container(iri("https://pod.ex/docs/"))));
        let v4 = store.policy_version(&target);
        assert_ne!(v3, v4, "revoking a document bumps the policy version");

        // A target NOT covered by the per-.acr is unaffected by its lifecycle.
        let other = iri("https://pod.ex/photos/y");
        let o1 = store.policy_version(&other);
        store
            .put(TrustDocument::new(
                TrustLayer::Container(iri("https://pod.ex/docs/")),
                3,
                mk("https://pod.ex/docs/"),
                gate(),
            ))
            .unwrap();
        let o2 = store.policy_version(&other);
        assert_eq!(
            o1, o2,
            "an unrelated container's edits don't affect this target"
        );
    }

    #[test]
    fn revoke_reports_presence() {
        let mut store = TrustStore::new();
        assert!(!store.revoke(&TrustLayer::ServerWide), "nothing to revoke");
        let key = a_key_hex();
        let rules = policy_rules(
            "https://pod.ex/trust#r",
            "https://gov.example/issuer",
            &key,
            "http://schema.org/age",
            "https://pod.ex/",
            "P30D",
        );
        store
            .put(TrustDocument::new(TrustLayer::ServerWide, 1, rules, gate()))
            .unwrap();
        assert!(store.revoke(&TrustLayer::ServerWide), "removed");
        assert!(!store.revoke(&TrustLayer::ServerWide), "already gone");
    }

    // ── Direct coverage tests for new public fns (sq-pfae.16) ────────────────

    /// Direct unit test: `TrustStore::effective_rules_at` with an explicit `now_secs`.
    /// Verifies the fn is reachable (direct line coverage) and that its output matches
    /// `effective_rules` (both should agree when rules are not time-gated, since the
    /// only difference is the wall-clock source).  Also exercises `now_unix_secs` via
    /// the `effective_rules` code path (system clock; we only check it is > 0).
    #[cfg(feature = "store")]
    #[test]
    fn effective_rules_at_is_directly_exercised() {
        let key = a_key_hex();
        let rules = policy_rules(
            "https://pod.ex/trust#cov-r",
            "https://gov.example/issuer",
            &key,
            "http://schema.org/age",
            "https://pod.ex/",
            "P30D",
        );
        let mut store = TrustStore::new();
        store
            .put(TrustDocument::new(
                TrustLayer::ServerWide,
                1,
                rules,
                gate(),
            ))
            .unwrap();
        let target = iri("https://pod.ex/docs/resource");
        // Direct call with an explicit instant — exercises the `effective_rules_at` body.
        let eff_at = store.effective_rules_at(&target, 1_700_000_000);
        assert_eq!(eff_at.len(), 1, "one direct rule");
        assert_eq!(eff_at[0].fresh_within_secs, 30 * 86_400);
        // `effective_rules` (system-clock path) must produce the same count for a
        // non-time-gated store — also exercises `now_unix_secs`.
        let eff = store.effective_rules(&target);
        assert_eq!(eff.len(), 1, "effective_rules agrees with effective_rules_at");
    }

    /// Direct unit test: `TrustDocument::with_certifications` constructor and
    /// `TrustDocument::certifications()` accessor (`cert-graph` feature).
    /// Verifies both are reachable (direct line coverage) and that the stored slice is
    /// returned verbatim (round-trip identity).
    #[cfg(feature = "cert-graph")]
    #[test]
    fn with_certifications_and_certifications_accessor_are_directly_exercised() {
        use crate::graph::{CertScope, Certification};
        use sparq_zk::sig::{SecretKey, public_key_to_hex};
        let key = a_key_hex();
        let rules = policy_rules(
            "https://pod.ex/trust#cov-r2",
            "https://gov.example/issuer",
            &key,
            "http://schema.org/age",
            "https://pod.ex/",
            "P30D",
        );
        let sk = SecretKey::from_seed(0xC0FF);
        let pk = sk.public_key();
        let dvs_sk = SecretKey::from_seed(0xBEEF);
        let dvs_pk = dvs_sk.public_key();
        let certifier_iri = iri("https://gov.example/framework");
        let certified_iri = iri("https://dvs.example/issuer");
        let mut cert = Certification {
            certifier: certifier_iri.clone(),
            certifier_key: pk,
            certified_issuer: certified_iri.clone(),
            certified_key: dvs_pk,
            scope: CertScope::AnyService,
            valid_from_unix_secs: 0,
            valid_until_unix_secs: i64::MAX / 2,
            signature_hex: String::new(),
        };
        let _ = public_key_to_hex(&sk.public_key()); // suppress unused import
        cert.signature_hex = sk.sign_commitment(&crate::graph::certification_message(&cert));

        // Exercise `with_certifications` directly.
        let doc = TrustDocument::with_certifications(
            TrustLayer::ServerWide,
            1,
            rules,
            vec![cert.clone()],
            gate(),
        );

        // Exercise `certifications()` directly — verify round-trip identity.
        let stored = doc.certifications();
        assert_eq!(stored.len(), 1, "one certification stored");
        assert_eq!(
            stored[0].certifier.as_str(),
            certifier_iri.as_str(),
            "certifier IRI round-trips"
        );
        assert_eq!(
            stored[0].certified_issuer.as_str(),
            certified_iri.as_str(),
            "certified_issuer IRI round-trips"
        );
    }
}
