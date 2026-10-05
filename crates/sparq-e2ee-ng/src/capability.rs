//! **Capabilities** (§4.2, §8.2): bearer secrets that encode *possession =
//! authority*. The profile keeps three authorities strictly separate, and this
//! module encodes that separation in the type + validation:
//!
//! * a **read** capability carries the branch read secret `K_read` (and permits
//!   intended decryption + topic subscription);
//! * a **publish** (write) capability additionally carries a publisher private
//!   key `sk_publish` (and registers `pk_publish` with the broker) — it does
//!   **not** carry admin authority;
//! * an **admin** capability carries a *distinct* `sk_admin` and never reuses
//!   `K_read` or `sk_publish`.
//!
//! A capability has a **canonical CBOR** representation (deterministic, so the
//! `cap_id` is stable) split into a **public grant** (safe to hand a broker for
//! admission, and the thing an admin signature authenticates) and **secret
//! fields** (`K_read`, private keys) that MUST be recipient-wrapped
//! ([`crate::wrap`]) or moved out of band — never placed in RDF, logs, URLs, or
//! topic messages (§4.2). [`wrap_capability`] / [`unwrap_capability`] are the
//! typed form of that recipient-wrapped path, under a fixed domain-separation
//! AAD ([`CAPABILITY_WRAP_AAD`]).
//!
//! A grant is scoped to a **branch set** ([`ScopedBranch`]) — usually one
//! branch, since a read secret and a topic are per-branch — and [`delegate`]
//! may narrow that set along with the authority set, validity window, and epoch
//! ceiling (§4.2), never widen any of them.

use crate::cbor::{
    enc_array, enc_bytes, enc_map, enc_text, enc_uint, read_struct_map, Limits, Reader,
};
use crate::error::{Error, Result};
use crate::ids::{BranchId, CapId, Epoch, RepoId, Secret32, TopicId};
use crate::sign::{PublicVerifyingKey, SecretSigningKey, PUBLIC_KEY_LEN, SIGNATURE_LEN};
use crate::suite::{check_suite, SUITE_V0};
use crate::wrap::{self, RecipientPublicKey, RecipientSecretKey, WrappedSecret};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// One of the three separated authorities (§4.2). The set carried by a
/// capability is canonicalized to this order (`read < publish < admin`) with no
/// duplicates, so its CBOR is deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Authority {
    /// Decrypt branch blocks and subscribe to its topic.
    Read,
    /// Publish signed commits to the branch topic.
    Publish,
    /// Create/rotate branches, change the publisher set, sign epoch transitions.
    Admin,
}

impl Authority {
    fn as_str(self) -> &'static str {
        match self {
            Authority::Read => "read",
            Authority::Publish => "publish",
            Authority::Admin => "admin",
        }
    }
    fn from_token(s: &str) -> Result<Self> {
        match s {
            "read" => Ok(Authority::Read),
            "publish" => Ok(Authority::Publish),
            "admin" => Ok(Authority::Admin),
            _ => Err(Error::Schema("unknown authority token")),
        }
    }
}

/// Canonicalize an authority set: sorted, deduplicated.
fn canon_authorities(mut v: Vec<Authority>) -> Vec<Authority> {
    v.sort();
    v.dedup();
    v
}

/// Validity window (§8.2 field 7): unix-second bounds. `not_before <= not_after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Validity {
    /// Not valid before this unix second.
    pub not_before: u64,
    /// Not valid after this unix second.
    pub not_after: u64,
}

/// One branch in a grant's **branch set** (§4.2), paired with the
/// epoch-specific [`TopicId`] that routes it. A topic is epoch-specific to *one*
/// branch (§4.1), so a branch is never in scope without the topic it is reached
/// by — carrying the pair keeps a multi-branch grant self-consistent instead of
/// naming branches it has no routing material for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopedBranch {
    /// The branch in scope.
    pub branch: BranchId,
    /// That branch's epoch-specific topic, for the grant's [`PublicGrant::epoch`].
    pub topic: TopicId,
}

/// Canonicalize a branch set: ascending by branch id, rejecting a degenerate
/// set. The result's first entry is the grant's *primary* branch (§8.2 fields
/// 3 + 5) and the rest go in the extra-branch field.
fn canon_scope(mut scope: Vec<ScopedBranch>) -> Result<Vec<ScopedBranch>> {
    if scope.is_empty() {
        return Err(Error::Schema("branch set must not be empty"));
    }
    scope.sort_by_key(|e| e.branch);
    check_scope(&scope)?;
    Ok(scope)
}

/// Fail-closed checks on an already-ordered branch set (primary entry first):
///
/// * **strictly ascending** branch ids — this both deduplicates and pins exactly
///   one byte encoding per logical branch set, which is what `cap_id` and the
///   admin signature rest on;
/// * **pairwise-distinct topics** — an epoch-specific topic identifies one
///   branch (§4.1), so two branches sharing one is a malformed grant rather than
///   a narrow one.
fn check_scope(scope: &[ScopedBranch]) -> Result<()> {
    if scope.windows(2).any(|w| w[1].branch <= w[0].branch) {
        return Err(Error::NonCanonical(
            "branch set not strictly ascending by branch id",
        ));
    }
    let mut topics: Vec<TopicId> = scope.iter().map(|e| e.topic).collect();
    topics.sort();
    if topics.windows(2).any(|w| w[0] == w[1]) {
        return Err(Error::Schema("two branches share one epoch-specific topic"));
    }
    Ok(())
}

/// The **public grant** — everything a broker may see and the exact bytes an
/// admin signature authenticates. Contains no secret key material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicGrant {
    /// Repository identifier (§8.2 field 2).
    pub repo: RepoId,
    /// **Primary** branch identifier (§8.2 field 3) — the lowest-ordered branch
    /// of the grant's branch set, and the branch [`Self::topic`] and a
    /// [`Capability::read_secret`] belong to. For a single-branch grant (the
    /// common case, and the only shape the profile's own §8.2 example shows)
    /// this is simply *the* branch.
    pub branch: BranchId,
    /// Epoch this grant is scoped to (§8.2 field 4).
    pub epoch: Epoch,
    /// Epoch-specific topic (§8.2 field 5) of [`Self::branch`].
    pub topic: TopicId,
    /// Authorities granted (§8.2 field 6), canonicalized.
    pub authority: Vec<Authority>,
    /// Validity window (§8.2 field 7).
    pub validity: Validity,
    /// Broker locators (§8.2 field 8).
    pub brokers: Vec<String>,
    /// Bound suite id (§8.2 field 9); MUST be the deployment's reviewed suite.
    pub suite: String,
    /// Random capability nonce (§8.2 field 13); makes the `cap_id` unlinkable to
    /// the grant's semantic fields.
    pub cap_nonce: [u8; 32],
    /// Publisher public key (profile field 15), present iff `publish` is granted.
    pub publisher_pub: Option<[u8; PUBLIC_KEY_LEN]>,
    /// Parent grant id for a delegated grant (profile field 16).
    pub parent_grant_id: Option<CapId>,
    /// Optional **maximum epoch** ceiling (profile field 17): the last epoch this
    /// grant may be exercised in. This is a *separate* bound from [`Self::epoch`]
    /// — `epoch` is the exact epoch (and epoch-specific [`Self::topic`]) the grant
    /// is scoped to, whereas `max_epoch` bounds how far a delegated chain may be
    /// carried forward. `None` means unbounded; when set it MUST be `>= epoch`.
    pub max_epoch: Option<Epoch>,
    /// The branch set's **additional** branches beyond [`Self::branch`] (profile
    /// field 18), each with its epoch-specific topic. Empty (and omitted from
    /// the wire) for a single-branch grant, so a single-branch grant encodes
    /// byte-for-byte as it did before the branch set existed.
    ///
    /// This must already be **canonical**: strictly ascending by branch id and
    /// every entry greater than [`Self::branch`], with topics distinct across
    /// the whole set. Use [`Self::set_branch_scope`] to establish that (and
    /// [`Self::branch_scope`] to read the whole set back) rather than assigning
    /// here; a non-canonical set is rejected by [`Self::decode`].
    ///
    /// The set is an **authorization** constraint on the admin-signed grant. It
    /// is not key distribution: a [`Capability`] still carries exactly one
    /// branch read secret (§8.2 field 10), so acting on the other branches
    /// needs their key material delivered separately — per §4.2, "cryptographic
    /// key possession remains necessary; the signed grant alone is not a
    /// decryption key".
    pub extra_branches: Vec<ScopedBranch>,
    /// Admin signature over the canonical public grant (§8.2 field 14).
    pub admin_sig: Option<[u8; SIGNATURE_LEN]>,
}

// wire keys (§8.2 + profile allocations 15/16/17)
const K_VERSION: u64 = 1;
const K_REPO: u64 = 2;
const K_BRANCH: u64 = 3;
const K_EPOCH: u64 = 4;
const K_TOPIC: u64 = 5;
const K_AUTHORITY: u64 = 6;
const K_VALIDITY: u64 = 7;
const K_BROKERS: u64 = 8;
const K_SUITE: u64 = 9;
const K_READ_SECRET: u64 = 10;
const K_PUBLISHER_SK: u64 = 11;
const K_ADMIN_SK: u64 = 12;
const K_CAP_NONCE: u64 = 13;
const K_ADMIN_SIG: u64 = 14;
const K_PUBLISHER_PK: u64 = 15;
const K_PARENT_GRANT: u64 = 16;
const K_MAX_EPOCH: u64 = 17;
const K_EXTRA_BRANCHES: u64 = 18;
const VERSION: u64 = 0;

// validity sub-map keys
const K_NB: u64 = 1;
const K_NA: u64 = 2;

// extra-branch entry sub-map keys
const K_SB_BRANCH: u64 = 1;
const K_SB_TOPIC: u64 = 2;

impl PublicGrant {
    /// The canonical public-grant fields **excluding** the admin signature
    /// (field 14). These are the exact bytes the admin signs and the `cap_id`
    /// hashes.
    fn signing_entries(&self) -> Vec<(u64, Vec<u8>)> {
        let mut e = vec![
            (K_VERSION, enc_uint(VERSION)),
            (K_REPO, enc_bytes(self.repo.as_bytes())),
            (K_BRANCH, enc_bytes(self.branch.as_bytes())),
            (K_EPOCH, enc_uint(self.epoch.0)),
            (K_TOPIC, enc_bytes(self.topic.as_bytes())),
            (
                K_AUTHORITY,
                enc_array(
                    &canon_authorities(self.authority.clone())
                        .iter()
                        .map(|a| enc_text(a.as_str()))
                        .collect::<Vec<_>>(),
                ),
            ),
            (
                K_VALIDITY,
                enc_map(vec![
                    (K_NB, enc_uint(self.validity.not_before)),
                    (K_NA, enc_uint(self.validity.not_after)),
                ]),
            ),
            (
                K_BROKERS,
                enc_array(&self.brokers.iter().map(|b| enc_text(b)).collect::<Vec<_>>()),
            ),
            (K_SUITE, enc_text(&self.suite)),
            (K_CAP_NONCE, enc_bytes(&self.cap_nonce)),
        ];
        if let Some(pk) = &self.publisher_pub {
            e.push((K_PUBLISHER_PK, enc_bytes(pk)));
        }
        if let Some(p) = &self.parent_grant_id {
            e.push((K_PARENT_GRANT, enc_bytes(p.as_bytes())));
        }
        if let Some(m) = &self.max_epoch {
            e.push((K_MAX_EPOCH, enc_uint(m.0)));
        }
        // Omitted entirely when the branch set is just the primary branch, so a
        // single-branch grant's bytes (and therefore its cap_id and every golden
        // vector) are unchanged. Unlike the authority array this is emitted
        // verbatim rather than re-canonicalized: an authority token is an atom,
        // so sorting/deduplicating it preserves meaning, whereas each branch
        // entry binds a topic — silently reordering or dropping one would
        // silently change which topic the grant binds to which branch. A
        // non-canonical set is therefore validated (and rejected) on decode,
        // never rewritten here.
        if !self.extra_branches.is_empty() {
            e.push((
                K_EXTRA_BRANCHES,
                enc_array(
                    &self
                        .extra_branches
                        .iter()
                        .map(|s| {
                            enc_map(vec![
                                (K_SB_BRANCH, enc_bytes(s.branch.as_bytes())),
                                (K_SB_TOPIC, enc_bytes(s.topic.as_bytes())),
                            ])
                        })
                        .collect::<Vec<_>>(),
                ),
            ));
        }
        e
    }

    /// The grant's whole **branch set** (§4.2) in canonical order: the primary
    /// branch entry ([`Self::branch`] + [`Self::topic`]) followed by
    /// [`Self::extra_branches`]. A single-branch grant yields one entry.
    pub fn branch_scope(&self) -> Vec<ScopedBranch> {
        let mut v = Vec::with_capacity(1 + self.extra_branches.len());
        v.push(ScopedBranch { branch: self.branch, topic: self.topic });
        v.extend_from_slice(&self.extra_branches);
        v
    }

    /// Whether `branch` is in this grant's branch set — the membership test an
    /// authorization check makes before honouring the grant for a branch.
    pub fn covers_branch(&self, branch: &BranchId) -> bool {
        self.branch == *branch || self.extra_branches.iter().any(|e| e.branch == *branch)
    }

    /// Replace the grant's branch set with `scope`, canonicalizing it: the
    /// lowest-ordered entry becomes the primary branch/topic and the rest become
    /// [`Self::extra_branches`] in ascending order.
    ///
    /// Fails closed on an empty set, a repeated branch, or two branches sharing
    /// one epoch-specific topic. The grant's admin signature (if any) is **not**
    /// refreshed — re-[`sign`](Self::sign) after changing the scope.
    pub fn set_branch_scope(&mut self, scope: Vec<ScopedBranch>) -> Result<()> {
        let canon = canon_scope(scope)?;
        let Some((primary, rest)) = canon.split_first() else {
            return Err(Error::Schema("branch set must not be empty"));
        };
        self.branch = primary.branch;
        self.topic = primary.topic;
        self.extra_branches = rest.to_vec();
        Ok(())
    }

    /// The exact bytes the admin signs / the `cap_id` hashes.
    pub fn signing_bytes(&self) -> Vec<u8> {
        enc_map(self.signing_entries())
    }

    /// Local lookup handle `cap_id = SHA-256(canonical public grant)` (§4.2). The
    /// secret-bearing serialization is never used as an identifier.
    pub fn cap_id(&self) -> CapId {
        let mut h = Sha256::new();
        h.update(self.signing_bytes());
        CapId::from_bytes(h.finalize().into())
    }

    /// Canonical CBOR of the public grant, including the admin signature when set.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = self.signing_entries();
        if let Some(sig) = &self.admin_sig {
            e.push((K_ADMIN_SIG, enc_bytes(sig)));
        }
        enc_map(e)
    }

    /// Sign this grant with an admin key, setting `admin_sig`.
    pub fn sign(&mut self, admin: &SecretSigningKey) {
        let sig = admin.sign(&self.signing_bytes());
        self.admin_sig = Some(sig);
    }

    /// Verify the admin signature against a trusted admin public key.
    pub fn verify(&self, admin_pub: &PublicVerifyingKey) -> Result<()> {
        let sig = self.admin_sig.ok_or(Error::Schema("missing admin signature"))?;
        admin_pub.verify(&self.signing_bytes(), &sig)
    }

    /// Decode + validate a public grant (fail-closed). Rejects an unknown suite,
    /// any secret field (10/11/12 are not permitted in a public grant), unknown
    /// mandatory fields, non-canonical CBOR, and an inconsistent authority set.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        let mut r = Reader::new(bytes, limits);
        let g = Self::decode_from(&mut r, false)?;
        r.finish()?;
        Ok(g)
    }

    /// Decode the public fields from an in-progress reader. `allow_secret`
    /// controls whether secret keys (10/11/12) are tolerated (they are consumed
    /// by [`Capability::decode`], never here).
    fn decode_from(r: &mut Reader<'_>, allow_secret: bool) -> Result<Self> {
        let mut version = None;
        let mut repo = None;
        let mut branch = None;
        let mut epoch = None;
        let mut topic = None;
        let mut authority: Option<Vec<Authority>> = None;
        let mut validity = None;
        let mut brokers: Option<Vec<String>> = None;
        let mut suite: Option<String> = None;
        let mut cap_nonce = None;
        let mut publisher_pub = None;
        let mut parent = None;
        let mut max_epoch = None;
        let mut extra_branches: Option<Vec<ScopedBranch>> = None;
        let mut admin_sig = None;

        read_struct_map(r, |r, key| match key {
            K_VERSION => {
                version = Some(r.uint()?);
                Ok(true)
            }
            K_REPO => {
                repo = Some(RepoId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            K_BRANCH => {
                branch = Some(BranchId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            K_EPOCH => {
                epoch = Some(Epoch(r.uint()?));
                Ok(true)
            }
            K_TOPIC => {
                topic = Some(TopicId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            K_AUTHORITY => {
                let n = r.array_header()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(Authority::from_token(r.text()?)?);
                }
                authority = Some(v);
                Ok(true)
            }
            K_VALIDITY => {
                let mut nb = None;
                let mut na = None;
                read_struct_map(r, |r, k| match k {
                    K_NB => {
                        nb = Some(r.uint()?);
                        Ok(true)
                    }
                    K_NA => {
                        na = Some(r.uint()?);
                        Ok(true)
                    }
                    _ => Ok(false),
                })?;
                validity = Some(Validity {
                    not_before: nb.ok_or(Error::Schema("missing not_before"))?,
                    not_after: na.ok_or(Error::Schema("missing not_after"))?,
                });
                Ok(true)
            }
            K_BROKERS => {
                let n = r.array_header()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(r.text()?.to_owned());
                }
                brokers = Some(v);
                Ok(true)
            }
            K_SUITE => {
                suite = Some(r.text()?.to_owned());
                Ok(true)
            }
            K_CAP_NONCE => {
                cap_nonce = Some(r.bytes_fixed::<32>()?);
                Ok(true)
            }
            K_PUBLISHER_PK => {
                publisher_pub = Some(r.bytes_fixed::<PUBLIC_KEY_LEN>()?);
                Ok(true)
            }
            K_PARENT_GRANT => {
                parent = Some(CapId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            K_MAX_EPOCH => {
                max_epoch = Some(Epoch(r.uint()?));
                Ok(true)
            }
            K_EXTRA_BRANCHES => {
                let n = r.array_header()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut b = None;
                    let mut t = None;
                    read_struct_map(r, |r, k| match k {
                        K_SB_BRANCH => {
                            b = Some(BranchId::from_bytes(r.bytes_fixed::<32>()?));
                            Ok(true)
                        }
                        K_SB_TOPIC => {
                            t = Some(TopicId::from_bytes(r.bytes_fixed::<32>()?));
                            Ok(true)
                        }
                        _ => Ok(false),
                    })?;
                    v.push(ScopedBranch {
                        branch: b.ok_or(Error::Schema("missing branch in branch-set entry"))?,
                        topic: t.ok_or(Error::Schema("missing topic in branch-set entry"))?,
                    });
                }
                extra_branches = Some(v);
                Ok(true)
            }
            K_ADMIN_SIG => {
                admin_sig = Some(r.bytes_fixed::<SIGNATURE_LEN>()?);
                Ok(true)
            }
            // Secret keys: consumed by Capability::decode, never valid in a
            // standalone public grant.
            K_READ_SECRET | K_PUBLISHER_SK | K_ADMIN_SK if allow_secret => {
                // Let the caller re-read; signal "recognized but skip here".
                r.skip_value()?;
                Ok(true)
            }
            _ => Ok(false),
        })?;

        if version != Some(VERSION) {
            return Err(Error::Schema("capability version"));
        }
        // The wire authority array must ALREADY be canonical (sorted, deduplicated):
        // normalizing here would accept multiple byte encodings of one logical
        // grant, breaking the bytes-level contract behind cap_id / admin_sig.
        // This is a *bytes-level* check, so it stays here rather than in
        // `validate_structure`: an in-memory array is re-canonicalized by
        // `signing_entries`, so only the wire form can be non-canonical.
        let authority = authority.ok_or(Error::Schema("missing authority"))?;
        if authority != canon_authorities(authority.clone()) {
            return Err(Error::NonCanonical("authority array not sorted/deduplicated"));
        }
        // Likewise bytes-level: an ABSENT field 18 is the single-branch form, so
        // an empty array would be a second encoding of one logical grant —
        // reject it rather than normalize, or `cap_id` stops being a function of
        // the grant. (An in-memory empty `extra_branches` IS the single-branch
        // form and is omitted by `signing_entries`.)
        let extra_branches = match extra_branches {
            Some(v) if v.is_empty() => {
                return Err(Error::NonCanonical(
                    "empty extra-branch array; omit the field for a single-branch grant",
                ))
            }
            Some(v) => v,
            None => Vec::new(),
        };
        let g = PublicGrant {
            repo: repo.ok_or(Error::Schema("missing repo"))?,
            branch: branch.ok_or(Error::Schema("missing branch"))?,
            epoch: epoch.ok_or(Error::Schema("missing epoch"))?,
            topic: topic.ok_or(Error::Schema("missing topic"))?,
            authority,
            validity: validity.ok_or(Error::Schema("missing validity"))?,
            brokers: brokers.ok_or(Error::Schema("missing brokers"))?,
            suite: suite.ok_or(Error::Schema("missing suite"))?,
            cap_nonce: cap_nonce.ok_or(Error::Schema("missing cap_nonce"))?,
            publisher_pub,
            parent_grant_id: parent,
            max_epoch,
            extra_branches,
            admin_sig,
        };
        g.validate_structure()?;
        Ok(g)
    }

    /// The structural invariants of a well-formed grant, checked on the
    /// **in-memory** value rather than on its wire bytes.
    ///
    /// This is the shared routine behind both directions: [`Self::decode_from`]
    /// runs it on every decoded grant, and [`wrap_capability`] runs it on a
    /// caller-constructed one, so a grant that a caller assembled field-by-field
    /// (every [`PublicGrant`] field is public) cannot be wrapped into bytes that
    /// its intended recipient would then be forced to reject.
    ///
    /// The two *bytes-level* canonicality rules — a sorted/deduplicated wire
    /// authority array and an omitted-not-empty extra-branch field — are
    /// deliberately NOT here: `signing_entries` establishes both when it
    /// encodes, so they can only be violated by a wire encoding, and
    /// [`Self::decode_from`] checks them directly.
    pub(crate) fn validate_structure(&self) -> Result<()> {
        check_suite(&self.suite)?;
        if self.validity.not_before > self.validity.not_after {
            return Err(Error::Schema("validity not_before > not_after"));
        }
        // The publisher public key is present iff publish is granted: a grant
        // without publish authority must not carry (and have authenticated) an
        // unrelated publisher key.
        if self.authority.contains(&Authority::Publish) != self.publisher_pub.is_some() {
            return Err(Error::Separation("publisher key presence must match publish authority"));
        }
        // The ceiling is a forward bound on the grant's own scope: a grant whose
        // max_epoch precedes the epoch it is scoped to could never be exercised,
        // so it is a malformed grant rather than a maximally-narrow one.
        if self.max_epoch.is_some_and(|m| m.0 < self.epoch.0) {
            return Err(Error::Schema("max_epoch precedes the grant epoch"));
        }
        // Validated over the WHOLE set (primary entry first), so this also pins
        // the primary branch as the lowest-ordered one: the same branch set can
        // therefore only be encoded one way.
        if !self.extra_branches.is_empty() {
            check_scope(&self.branch_scope())?;
        }
        Ok(())
    }
}

/// Constraints a delegated grant may narrow (never widen) relative to its parent
/// (§4.2): the **branch set**, the authority set, the validity window, and an
/// optional maximum epoch may only shrink.
#[derive(Debug, Clone)]
pub struct Delegation {
    /// Branches the child may act on — MUST be a non-empty **subset** of the
    /// parent's branch set. `None` inherits the parent's set unchanged, so a
    /// delegation can never reach a branch the parent was not already scoped to.
    /// Each retained branch keeps the parent's epoch-specific topic for it; a
    /// delegation never invents routing material.
    pub branches: Option<Vec<BranchId>>,
    /// Authorities the child may exercise — MUST be a subset of the parent's.
    pub authority: Vec<Authority>,
    /// Child validity window — MUST be within the parent's.
    pub validity: Validity,
    /// Optional maximum epoch the child may be exercised in — MUST NOT exceed a
    /// ceiling the parent is already under, and MUST NOT precede the epoch the
    /// grant is scoped to. Leaving it `None` inherits the parent's ceiling. This
    /// narrows the grant's *forward* extent; it never rewrites the exact epoch
    /// (and epoch-specific topic) the child inherits from its parent.
    pub max_epoch: Option<u64>,
}

/// A full capability, optionally bearing secrets. The secret fields never appear
/// in the public grant; serialize them only via [`Capability::encode_secret`]
/// for recipient-wrapped / out-of-band transfer — or, for the recipient-wrapped
/// case, prefer [`wrap_capability`] / [`unwrap_capability`], which do that under a
/// fixed domain-separation AAD without exposing the plaintext to the caller.
///
/// Its [`Debug`](core::fmt::Debug) is hand-written and redacts every secret
/// field — see the impl below.
pub struct Capability {
    /// The public, signable grant.
    pub grant: PublicGrant,
    /// Branch read secret `K_read` (§8.2 field 10) — present for a read+ capability.
    pub read_secret: Option<Secret32>,
    /// Publisher private key seed (§8.2 field 11) — present for a write capability.
    pub publisher_sk: Option<[u8; 32]>,
    /// Admin private key seed (§8.2 field 12) — present for an admin capability.
    pub admin_sk: Option<[u8; 32]>,
}

/// Debug stand-in for a secret field: prints `REDACTED` where the bytes would
/// go, so an `Option<[u8; 32]>` seed renders as `Some(REDACTED)` / `None`.
struct Redacted;

impl core::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("REDACTED")
    }
}

/// Hand-written so `{:?}` NEVER prints private key material (§4.2: secret fields
/// must not reach logs). `read_secret` is a [`Secret32`] and redacts through its
/// own `Debug`, but `publisher_sk` / `admin_sk` are bare `[u8; 32]` seeds that a
/// derived `Debug` would print verbatim. Only their *presence* is shown, which
/// [`Capability::validate`] already ties one-to-one to the public grant's
/// authority set.
impl core::fmt::Debug for Capability {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Capability")
            .field("grant", &self.grant)
            .field("read_secret", &self.read_secret)
            .field("publisher_sk", &self.publisher_sk.as_ref().map(|_| Redacted))
            .field("admin_sk", &self.admin_sk.as_ref().map(|_| Redacted))
            .finish()
    }
}

impl Capability {
    /// Validate the read/publish/admin **separation** invariants (§4.2, §8.2):
    /// * a publisher private key and an admin private key are never combined;
    /// * `publish` authority ⟺ a publisher key pair is present, and the secret
    ///   key actually derives the grant's `publisher_pub` (so the key the
    ///   bearer signs with is the key the admin-signed grant / broker
    ///   registration binds);
    /// * `admin` authority ⟺ an admin private key is present;
    /// * `read` authority ⟺ a read secret is present.
    pub fn validate(&self) -> Result<()> {
        if self.publisher_sk.is_some() && self.admin_sk.is_some() {
            return Err(Error::Separation(
                "publisher and admin private keys must not be combined",
            ));
        }
        let auth = &self.grant.authority;
        if auth.contains(&Authority::Read) != self.read_secret.is_some() {
            return Err(Error::Separation("read authority <-> read secret mismatch"));
        }
        // Check the secret and public halves independently, so a non-publish
        // capability carrying only one of them (e.g. a stray publisher_pub) is
        // still rejected.
        let publish = auth.contains(&Authority::Publish);
        if publish != self.publisher_sk.is_some() {
            return Err(Error::Separation("publish authority <-> publisher secret mismatch"));
        }
        if publish != self.grant.publisher_pub.is_some() {
            return Err(Error::Separation("publish authority <-> publisher_pub mismatch"));
        }
        if let (Some(sk), Some(pk)) = (&self.publisher_sk, &self.grant.publisher_pub) {
            let derived = SecretSigningKey::from_seed(*sk).public().to_bytes();
            if !bool::from(derived.as_slice().ct_eq(pk.as_slice())) {
                return Err(Error::Separation(
                    "publisher secret does not derive the granted publisher_pub",
                ));
            }
        }
        if auth.contains(&Authority::Admin) != self.admin_sk.is_some() {
            return Err(Error::Separation("admin authority <-> admin key mismatch"));
        }
        Ok(())
    }

    /// Build a **read** capability (authority = {read}), carrying only `K_read`.
    pub fn new_read(grant_base: PublicGrant, read_secret: Secret32) -> Result<Self> {
        let mut grant = grant_base;
        grant.authority = vec![Authority::Read];
        grant.publisher_pub = None;
        let cap = Capability {
            grant,
            read_secret: Some(read_secret),
            publisher_sk: None,
            admin_sk: None,
        };
        cap.validate()?;
        Ok(cap)
    }

    /// Build a **write** (read+publish) capability, carrying `K_read` and a
    /// publisher key pair. Not admin.
    pub fn new_write(
        grant_base: PublicGrant,
        read_secret: Secret32,
        publisher: &SecretSigningKey,
    ) -> Result<Self> {
        let mut grant = grant_base;
        grant.authority = vec![Authority::Read, Authority::Publish];
        grant.publisher_pub = Some(publisher.public().to_bytes());
        let cap = Capability {
            grant,
            read_secret: Some(read_secret),
            publisher_sk: Some(publisher.to_seed()),
            admin_sk: None,
        };
        cap.validate()?;
        Ok(cap)
    }

    /// Build an **admin** capability, carrying a distinct `sk_admin`. It carries
    /// `admin` authority and MUST NOT reuse `K_read` or `sk_publish`.
    pub fn new_admin(grant_base: PublicGrant, admin: &SecretSigningKey) -> Result<Self> {
        let mut grant = grant_base;
        grant.authority = vec![Authority::Admin];
        grant.publisher_pub = None;
        let cap = Capability {
            grant,
            read_secret: None,
            publisher_sk: None,
            admin_sk: Some(admin.to_seed()),
        };
        cap.validate()?;
        Ok(cap)
    }

    /// Canonical CBOR including secret fields, for recipient-wrapped / out-of-band
    /// transfer ONLY. The result is a bearer secret; wrap it ([`crate::wrap`])
    /// before it leaves protected local storage.
    pub fn encode_secret(&self) -> Vec<u8> {
        let mut e = self.grant.signing_entries();
        if let Some(sig) = &self.grant.admin_sig {
            e.push((K_ADMIN_SIG, enc_bytes(sig)));
        }
        if let Some(rs) = &self.read_secret {
            e.push((K_READ_SECRET, enc_bytes(rs.expose())));
        }
        if let Some(sk) = &self.publisher_sk {
            e.push((K_PUBLISHER_SK, enc_bytes(sk)));
        }
        if let Some(sk) = &self.admin_sk {
            e.push((K_ADMIN_SK, enc_bytes(sk)));
        }
        enc_map(e)
    }

    /// Decode a full (possibly secret-bearing) capability and validate separation.
    pub fn decode_secret(bytes: &[u8], limits: Limits) -> Result<Self> {
        // We need the secret fields too, so decode the map twice: once for the
        // public grant (canonical validation), once to pull the secret fields.
        // A single pass keeps ordering checks intact.
        let mut r = Reader::new(bytes, limits);
        let mut read_secret: Option<[u8; 32]> = None;
        let mut publisher_sk: Option<[u8; 32]> = None;
        let mut admin_sk: Option<[u8; 32]> = None;

        // First pass over a cloned reader for the public grant.
        let grant = {
            let mut r2 = Reader::new(bytes, limits);
            let g = PublicGrant::decode_from(&mut r2, true)?;
            r2.finish()?;
            g
        };

        // Second pass to capture secrets (canonical order already validated).
        read_struct_map(&mut r, |r, key| match key {
            K_READ_SECRET => {
                read_secret = Some(r.bytes_fixed::<32>()?);
                Ok(true)
            }
            K_PUBLISHER_SK => {
                publisher_sk = Some(r.bytes_fixed::<32>()?);
                Ok(true)
            }
            K_ADMIN_SK => {
                admin_sk = Some(r.bytes_fixed::<32>()?);
                Ok(true)
            }
            // Skip every public field in this pass.
            K_VERSION | K_EPOCH | K_MAX_EPOCH => {
                r.uint()?;
                Ok(true)
            }
            K_REPO | K_BRANCH | K_TOPIC | K_CAP_NONCE | K_PUBLISHER_PK | K_PARENT_GRANT
            | K_ADMIN_SIG => {
                r.skip_value()?;
                Ok(true)
            }
            K_AUTHORITY | K_VALIDITY | K_BROKERS | K_SUITE | K_EXTRA_BRANCHES => {
                r.skip_value()?;
                Ok(true)
            }
            _ => Ok(false),
        })?;
        r.finish()?;

        let cap = Capability {
            grant,
            read_secret: read_secret.map(Secret32),
            publisher_sk,
            admin_sk,
        };
        cap.validate()?;
        Ok(cap)
    }

    /// The corresponding publisher signing key, if this is a write capability.
    pub fn publisher_key(&self) -> Option<SecretSigningKey> {
        self.publisher_sk.map(SecretSigningKey::from_seed)
    }

    /// The corresponding admin signing key, if this is an admin capability.
    pub fn admin_key(&self) -> Option<SecretSigningKey> {
        self.admin_sk.map(SecretSigningKey::from_seed)
    }

    /// Overwrite the raw private-key seeds in place. `read_secret` is a
    /// [`Secret32`] and zeroizes via its own `Drop`; these two fields are bare
    /// `[u8; 32]` seeds, so [`Drop`] routes through here to give the whole
    /// capability the same secret-memory hygiene.
    fn zeroize_secrets(&mut self) {
        use zeroize::Zeroize;
        if let Some(sk) = self.publisher_sk.as_mut() {
            sk.zeroize();
        }
        if let Some(sk) = self.admin_sk.as_mut() {
            sk.zeroize();
        }
    }
}

/// Zero the raw private-key seed bytes when a capability is dropped, so a decoded
/// or constructed write/admin capability does not leave `sk_publish` / `sk_admin`
/// in freed memory (Cargo.toml secret-memory hygiene). `read_secret` is a
/// [`Secret32`] and already zeroizes itself.
impl Drop for Capability {
    fn drop(&mut self) {
        self.zeroize_secrets();
    }
}

/// The fixed domain-separation AAD every capability wrapping is bound to.
///
/// [`wrap_capability`] / [`unwrap_capability`] always pass exactly these bytes as
/// the AEAD associated data, so a capability wrapping can never be opened as — or
/// substituted for — a wrapping made for another purpose (a per-object DEK, a bare
/// `K_read`, a future wrapped payload), each of which carries its own label under
/// the generic [`crate::wrap::wrap`]. The label is versioned like the key-schedule
/// labels (§8.3): a v1 capability wrapping would use a distinct one.
pub const CAPABILITY_WRAP_AAD: &[u8] = b"urn:jeswr:w3id:e2ee-ng:draft:2026-07 capability-wrap v0";

/// Recipient-wrap a capability **including its secret fields** — the recommended
/// secret-transfer path of §4.2.
///
/// This is [`Capability::encode_secret`] followed by [`crate::wrap::wrap`] under the
/// fixed [`CAPABILITY_WRAP_AAD`], with two things the hand-rolled two-step does not
/// give you: the caller never handles the bearer-secret plaintext (this helper
/// zeroizes its own copy before returning), and the AAD is not a caller-chosen
/// string that the two ends can disagree on.
///
/// The capability is validated first — both the read/publish/admin separation
/// invariants ([`Capability::validate`]) and the public grant's structural ones
/// (`PublicGrant::validate_structure`, the same routine
/// [`PublicGrant::decode`] runs) — so a malformed one fails here rather than
/// producing bytes that [`unwrap_capability`] would always reject. Every
/// [`PublicGrant`] field is public, so the grant half needs checking too:
/// separation-correct secrets can still sit on a grant carrying an unsupported
/// suite, an inverted validity window, a `max_epoch` behind its epoch, or a
/// non-canonical branch set.
///
/// Zeroizing covers this function's plaintext buffer; it does not retroactively
/// erase the intermediate CBOR fragments `encode_secret` allocates internally —
/// that exposure is unchanged from calling `encode_secret` directly.
///
/// ```
/// use sparq_e2ee_ng::capability::{
///     base_grant, unwrap_capability, wrap_capability, Capability, Validity,
/// };
/// use sparq_e2ee_ng::cbor::Limits;
/// use sparq_e2ee_ng::ids::{BranchId, Epoch, RepoId, Secret32, TopicId};
/// use sparq_e2ee_ng::wrap::RecipientSecretKey;
///
/// let grant = base_grant(
///     RepoId::random(),
///     BranchId::random(),
///     Epoch(0),
///     TopicId::random(),
///     Validity { not_before: 0, not_after: 100 },
///     vec!["wss://broker.example".to_string()],
/// );
/// let cap = Capability::new_read(grant, Secret32::random()).unwrap();
///
/// let recipient = RecipientSecretKey::generate();
/// let wrapped = wrap_capability(&cap, &recipient.public()).unwrap();
/// let opened = unwrap_capability(&recipient, &wrapped, Limits::default()).unwrap();
/// assert_eq!(opened.grant, cap.grant);
/// ```
pub fn wrap_capability(cap: &Capability, recipient: &RecipientPublicKey) -> Result<WrappedSecret> {
    use zeroize::Zeroize;
    cap.validate()?;
    cap.grant.validate_structure()?;
    let mut plaintext = cap.encode_secret();
    let wrapped = wrap::wrap(recipient, &plaintext, CAPABILITY_WRAP_AAD);
    plaintext.zeroize();
    wrapped
}

/// Open a capability wrapped by [`wrap_capability`], with the recipient's private
/// key.
///
/// Fails closed as [`Error::Decrypt`] on a wrong key, tampered ciphertext, or a
/// wrapping made under any other AAD (including one produced by the generic
/// [`crate::wrap::wrap`] with a caller-chosen label); the decrypted plaintext then
/// goes through [`Capability::decode_secret`], so canonical-encoding and
/// read/publish/admin separation are still enforced on untrusted input. The
/// decrypted plaintext buffer is zeroized before returning either way.
pub fn unwrap_capability(
    sk: &RecipientSecretKey,
    wrapped: &WrappedSecret,
    limits: Limits,
) -> Result<Capability> {
    use zeroize::Zeroize;
    let mut plaintext = wrap::unwrap(sk, wrapped, CAPABILITY_WRAP_AAD)?;
    let cap = Capability::decode_secret(&plaintext, limits);
    plaintext.zeroize();
    cap
}

/// Delegate a **new** public grant from `parent`, narrowing constraints, and
/// sign it with the admin key. The child's branch set MUST be a subset of the
/// parent's, its authority set a subset of the parent's, its validity window
/// within the parent's, and its `max_epoch` (if any) no greater than the
/// parent's ceiling. The child always keeps the parent's exact `epoch`, and each
/// branch it retains keeps that branch's epoch-specific topic — an epoch ceiling
/// is carried in the separate, admin-signed `max_epoch` field, never by
/// rewriting the epoch scope out from under a topic. Cryptographic key
/// possession is still required to act — a signed grant alone is not a
/// decryption key.
///
/// A `parent` whose own branch set is not canonical is rejected outright, on
/// either path, so a child is never admin-signed over a branch set that
/// [`PublicGrant::decode`] would then reject.
pub fn delegate(
    parent: &PublicGrant,
    admin: &SecretSigningKey,
    d: Delegation,
) -> Result<PublicGrant> {
    // The branch set narrows by SELECTION out of the parent's set: an entry is
    // only ever copied from the parent (with its topic), so a delegation cannot
    // name a branch the parent lacked, and re-delegation cannot recover a branch
    // an ancestor already dropped.
    let parent_scope = parent.branch_scope();
    // The parent's OWN set is validated first, before either branch below. Every
    // field of a `PublicGrant` is public, so a parent can reach here without ever
    // having been through `decode` / `set_branch_scope` — and the inheriting path
    // copies its set verbatim. Without this the child would inherit a
    // non-canonical set, get admin-signed over it (`signing_bytes` validates
    // nothing), and then `verify` on the child would succeed while
    // `PublicGrant::decode(child.encode())` rejects it: an authenticated but
    // structurally invalid grant. The error propagates unchanged rather than
    // being relabelled `Delegation`, since it is the parent that is malformed,
    // not this delegation request.
    check_scope(&parent_scope)?;
    let child_scope = match d.branches {
        None => parent_scope,
        Some(sel) => {
            if sel.is_empty() {
                return Err(Error::Delegation("branch set must not be empty"));
            }
            let mut out: Vec<ScopedBranch> = Vec::with_capacity(sel.len());
            for b in &sel {
                // A repeated branch is rejected rather than silently collapsed,
                // so the child's set is exactly the one that was asked for.
                if out.iter().any(|e| e.branch == *b) {
                    return Err(Error::Delegation("duplicate branch in branch set"));
                }
                match parent_scope.iter().find(|e| e.branch == *b) {
                    Some(e) => out.push(*e),
                    None => return Err(Error::Delegation("branch not in parent branch set")),
                }
            }
            // Sorts the selection into canonical order. Its remaining
            // fail-closed checks can only trip if the PARENT's own set was
            // malformed, so that error propagates unchanged rather than being
            // relabelled as a mistake in this delegation request.
            canon_scope(out)?
        }
    };
    let Some((primary, extra_branches)) = child_scope.split_first() else {
        return Err(Error::Delegation("branch set must not be empty"));
    };

    let child_auth = canon_authorities(d.authority);
    let parent_auth = canon_authorities(parent.authority.clone());
    for a in &child_auth {
        if !parent_auth.contains(a) {
            return Err(Error::Delegation("authority not a subset of parent"));
        }
    }
    if d.validity.not_before < parent.validity.not_before
        || d.validity.not_after > parent.validity.not_after
        || d.validity.not_before > d.validity.not_after
    {
        return Err(Error::Delegation("validity window widens parent"));
    }
    // The epoch ceiling is its own authenticated field: the child keeps the
    // parent's exact epoch scope (and therefore the parent's epoch-specific
    // topic), and narrows only how far forward the grant may be carried.
    let max_epoch = match (d.max_epoch, parent.max_epoch) {
        (Some(m), Some(p)) if m > p.0 => {
            return Err(Error::Delegation("max_epoch exceeds parent max_epoch"))
        }
        (Some(m), _) if m < parent.epoch.0 => {
            return Err(Error::Delegation("max_epoch precedes the grant epoch"))
        }
        (Some(m), _) => Some(Epoch(m)),
        // An unspecified ceiling inherits the parent's, so a delegation can
        // never escape a bound the parent is already under.
        (None, p) => p,
    };
    // A delegated grant does not itself carry the publisher key pair unless
    // publish is delegated AND the parent carried one to bind.
    let publisher_pub = if child_auth.contains(&Authority::Publish) {
        parent.publisher_pub
    } else {
        None
    };
    if child_auth.contains(&Authority::Publish) && publisher_pub.is_none() {
        return Err(Error::Delegation("cannot delegate publish without a publisher key"));
    }
    let mut child = PublicGrant {
        repo: parent.repo,
        branch: primary.branch,
        epoch: parent.epoch,
        topic: primary.topic,
        authority: child_auth,
        validity: d.validity,
        brokers: parent.brokers.clone(),
        suite: parent.suite.clone(),
        cap_nonce: Secret32::random().0, // fresh nonce so the child cap_id differs
        publisher_pub,
        parent_grant_id: Some(parent.cap_id()),
        max_epoch,
        extra_branches: extra_branches.to_vec(),
        admin_sig: None,
    };
    child.sign(admin);
    Ok(child)
}

/// Convenience constructor for a base public grant with a fresh random cap nonce.
/// The suite is fixed to the bound v0 suite; authority/publisher fields are set
/// by the [`Capability`] constructors. The branch set starts as the single
/// `branch` given here — widen it with [`PublicGrant::set_branch_scope`].
#[allow(clippy::too_many_arguments)]
pub fn base_grant(
    repo: RepoId,
    branch: BranchId,
    epoch: Epoch,
    topic: TopicId,
    validity: Validity,
    brokers: Vec<String>,
) -> PublicGrant {
    PublicGrant {
        repo,
        branch,
        epoch,
        topic,
        authority: vec![],
        validity,
        brokers,
        suite: SUITE_V0.to_string(),
        cap_nonce: Secret32::random().0,
        publisher_pub: None,
        parent_grant_id: None,
        max_epoch: None,
        extra_branches: Vec::new(),
        admin_sig: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a sample grant whose authority array carries exactly `tokens`,
    /// bypassing the canonicalization the honest encoder applies.
    fn grant_bytes_with_authority_tokens(tokens: &[&str]) -> Vec<u8> {
        let mut g = base_grant(
            RepoId::from_bytes([1u8; 32]),
            BranchId::from_bytes([2u8; 32]),
            Epoch(7),
            TopicId::from_bytes([3u8; 32]),
            Validity { not_before: 100, not_after: 200 },
            vec!["wss://broker.example".to_string()],
        );
        g.authority = vec![Authority::Read];
        let mut entries = g.signing_entries();
        let auth = enc_array(&tokens.iter().map(|t| enc_text(t)).collect::<Vec<_>>());
        for e in &mut entries {
            if e.0 == K_AUTHORITY {
                e.1 = auth.clone();
            }
        }
        enc_map(entries)
    }

    #[test]
    fn decode_rejects_out_of_order_authority_array() {
        let bytes = grant_bytes_with_authority_tokens(&["publish", "read"]);
        assert!(matches!(
            PublicGrant::decode(&bytes, Limits::default()),
            Err(Error::NonCanonical(_))
        ));
    }

    #[test]
    fn decode_rejects_duplicate_authority_array() {
        let bytes = grant_bytes_with_authority_tokens(&["read", "read"]);
        assert!(matches!(
            PublicGrant::decode(&bytes, Limits::default()),
            Err(Error::NonCanonical(_))
        ));
    }

    #[test]
    fn decode_accepts_canonical_authority_array() {
        let bytes = grant_bytes_with_authority_tokens(&["read"]);
        let g = PublicGrant::decode(&bytes, Limits::default()).unwrap();
        assert_eq!(g.authority, vec![Authority::Read]);
    }

    /// The `Drop` path must overwrite the raw private-key seeds. Observed via the
    /// same `zeroize_secrets` routine `Drop` calls, so a future field change that
    /// silently drops the guarantee fails here. `read_secret` is a `Secret32` and
    /// zeroizes through its own `Drop`.
    #[test]
    fn drop_zeroizes_private_key_seeds() {
        let base = base_grant(
            RepoId::from_bytes([1u8; 32]),
            BranchId::from_bytes([2u8; 32]),
            Epoch(7),
            TopicId::from_bytes([3u8; 32]),
            Validity { not_before: 100, not_after: 200 },
            vec!["wss://broker.example".to_string()],
        );

        let publisher = SecretSigningKey::from_seed([7u8; 32]);
        let mut write = Capability::new_write(base.clone(), Secret32([9u8; 32]), &publisher).unwrap();
        assert_eq!(write.publisher_sk, Some([7u8; 32]));
        write.zeroize_secrets();
        assert_eq!(write.publisher_sk, Some([0u8; 32]));

        let admin = SecretSigningKey::from_seed([5u8; 32]);
        let mut admin_cap = Capability::new_admin(base, &admin).unwrap();
        assert_eq!(admin_cap.admin_sk, Some([5u8; 32]));
        admin_cap.zeroize_secrets();
        assert_eq!(admin_cap.admin_sk, Some([0u8; 32]));
    }

    /// `{:?}` must never disclose private key material: the two bare seed fields
    /// are redacted like `read_secret`'s `Secret32` already is, so a capability
    /// reaching a log/panic message leaks only which authorities it bears. A
    /// derived `Debug` would print the seed bytes verbatim and fail here.
    #[test]
    fn debug_redacts_private_key_seeds() {
        let mut base = base_grant(
            RepoId::from_bytes([1u8; 32]),
            BranchId::from_bytes([2u8; 32]),
            Epoch(7),
            TopicId::from_bytes([3u8; 32]),
            Validity { not_before: 100, not_after: 200 },
            vec!["wss://broker.example".to_string()],
        );
        // Pin the otherwise-random nonce so the whole rendering is deterministic
        // and the "no seed byte anywhere" assertions below cannot flake.
        base.cap_nonce = [4u8; 32];

        // A seed byte distinct from every public field's fill byte, so a hit in
        // the rendering can only have come from the seed itself.
        let publisher = SecretSigningKey::from_seed([0xAB; 32]);
        let write = Capability::new_write(base.clone(), Secret32([0xCD; 32]), &publisher).unwrap();
        let s = format!("{write:?}");
        assert!(s.contains("publisher_sk: Some(REDACTED)"), "{s}");
        assert!(s.contains("admin_sk: None"), "{s}");
        assert!(s.contains("read_secret: Some(Secret32(REDACTED))"), "{s}");
        assert!(!s.contains("171"), "publisher seed byte leaked: {s}");
        assert!(!s.contains("205"), "read secret byte leaked: {s}");

        let admin = SecretSigningKey::from_seed([0xAB; 32]);
        let admin_cap = Capability::new_admin(base, &admin).unwrap();
        let s = format!("{admin_cap:?}");
        assert!(s.contains("admin_sk: Some(REDACTED)"), "{s}");
        assert!(s.contains("publisher_sk: None"), "{s}");
        assert!(!s.contains("171"), "admin seed byte leaked: {s}");
    }

    /// The epoch ceiling is a forward bound, so a grant whose `max_epoch`
    /// precedes its own epoch scope is malformed and must fail closed on decode
    /// (it could never be exercised). A ceiling at or after the epoch decodes.
    #[test]
    fn decode_rejects_max_epoch_before_grant_epoch() {
        let mut g = base_grant(
            RepoId::from_bytes([1u8; 32]),
            BranchId::from_bytes([2u8; 32]),
            Epoch(7),
            TopicId::from_bytes([3u8; 32]),
            Validity { not_before: 100, not_after: 200 },
            vec!["wss://broker.example".to_string()],
        );
        g.authority = vec![Authority::Read];
        g.max_epoch = Some(Epoch(3));
        assert!(matches!(
            PublicGrant::decode(&g.encode(), Limits::default()),
            Err(Error::Schema(_))
        ));

        g.max_epoch = Some(Epoch(7));
        let ok = PublicGrant::decode(&g.encode(), Limits::default()).unwrap();
        assert_eq!(ok.max_epoch, Some(Epoch(7)));
        assert_eq!(ok.epoch, Epoch(7));
    }

    /// A grant whose extra-branch field is written by hand (bypassing
    /// [`PublicGrant::set_branch_scope`]) must fail closed on decode unless the
    /// whole branch set is canonical: strictly ascending from the primary entry,
    /// with distinct topics, and absent rather than empty when there is only one
    /// branch. Otherwise one logical branch set would have several encodings and
    /// `cap_id` / the admin signature would stop pinning it.
    #[test]
    fn decode_rejects_non_canonical_branch_set() {
        let scoped = |b: u8, t: u8| ScopedBranch {
            branch: BranchId::from_bytes([b; 32]),
            topic: TopicId::from_bytes([t; 32]),
        };
        let mut g = base_grant(
            RepoId::from_bytes([1u8; 32]),
            BranchId::from_bytes([0x20; 32]),
            Epoch(7),
            TopicId::from_bytes([0xA2; 32]),
            Validity { not_before: 100, not_after: 200 },
            vec!["wss://broker.example".to_string()],
        );
        g.authority = vec![Authority::Read];

        // Canonical: two extras, both above the primary, ascending, topics distinct.
        g.extra_branches = vec![scoped(0x30, 0xA3), scoped(0x40, 0xA4)];
        let ok = PublicGrant::decode(&g.encode(), Limits::default()).unwrap();
        assert_eq!(ok.branch_scope().len(), 3);
        assert!(ok.covers_branch(&BranchId::from_bytes([0x40; 32])));

        for bad in [
            // extras out of order
            vec![scoped(0x40, 0xA4), scoped(0x30, 0xA3)],
            // an extra repeating the primary branch
            vec![scoped(0x20, 0xA9)],
            // an extra below the primary (the primary must be the lowest)
            vec![scoped(0x10, 0xA1)],
            // a duplicated extra
            vec![scoped(0x30, 0xA3), scoped(0x30, 0xA9)],
        ] {
            g.extra_branches = bad;
            assert!(matches!(
                PublicGrant::decode(&g.encode(), Limits::default()),
                Err(Error::NonCanonical(_))
            ));
        }

        // A topic is epoch-specific to ONE branch: sharing one is malformed.
        g.extra_branches = vec![scoped(0x30, 0xA2)];
        assert!(matches!(
            PublicGrant::decode(&g.encode(), Limits::default()),
            Err(Error::Schema(_))
        ));

        // A present-but-empty array is a second encoding of the single-branch
        // form; only omitting the field is canonical.
        g.extra_branches = vec![];
        let mut entries = g.signing_entries();
        entries.push((K_EXTRA_BRANCHES, enc_array(&[])));
        assert!(matches!(
            PublicGrant::decode(&enc_map(entries), Limits::default()),
            Err(Error::NonCanonical(_))
        ));
    }

    /// A read-only grant carrying a publisher_pub violates the
    /// `publisher_pub iff publish` invariant on public decode.
    #[test]
    fn public_decode_rejects_publisher_pub_without_publish() {
        let mut g = base_grant(
            RepoId::from_bytes([1u8; 32]),
            BranchId::from_bytes([2u8; 32]),
            Epoch(7),
            TopicId::from_bytes([3u8; 32]),
            Validity { not_before: 100, not_after: 200 },
            vec!["wss://broker.example".to_string()],
        );
        g.authority = vec![Authority::Read];
        g.publisher_pub = Some([4u8; PUBLIC_KEY_LEN]);
        assert!(matches!(
            PublicGrant::decode(&g.encode(), Limits::default()),
            Err(Error::Separation(_))
        ));
    }

    /// Same reverse mismatch through the secret-bearing decode path: a read
    /// capability whose grant smuggles a publisher_pub (no publisher_sk).
    #[test]
    fn secret_decode_rejects_publisher_pub_without_publish() {
        let mut g = base_grant(
            RepoId::from_bytes([1u8; 32]),
            BranchId::from_bytes([2u8; 32]),
            Epoch(7),
            TopicId::from_bytes([3u8; 32]),
            Validity { not_before: 100, not_after: 200 },
            vec!["wss://broker.example".to_string()],
        );
        g.authority = vec![Authority::Read];
        g.publisher_pub = Some([4u8; PUBLIC_KEY_LEN]);
        let cap = Capability {
            grant: g,
            read_secret: Some(Secret32([9u8; 32])),
            publisher_sk: None,
            admin_sk: None,
        };
        assert!(matches!(
            Capability::decode_secret(&cap.encode_secret(), Limits::default()),
            Err(Error::Separation(_))
        ));
        // validate() itself must also catch it, independent of decoding.
        assert!(matches!(cap.validate(), Err(Error::Separation(_))));
    }
}
