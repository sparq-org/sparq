// AUTHORED-BY Claude Opus 4.8
//! Tiered proof-of-possession (PoP) — the auth-confirmation dispatch.
//!
//! Design: `research/lws-design-records.md` §7 (RSS `docs/design/high-throughput-pop-auth.md`, left
//! behind by the `sq-gg0qq.2` import — these module docs ARE its in-repo home). DPoP (RFC 9449)
//! stays the **mandatory** Solid-OIDC baseline; the tiers here are negotiated, opt-in fast paths
//! that are *individually refusable* and never a silent downgrade.
//!
//! ## What this module is (design increment **T1a**)
//! The foundation the rest of Tier 1 builds on: given a token's **confirmation method**
//! (`Confirmation`) and (for the cert-bound method) the client certificate presented on the TLS
//! connection, `dispatch` routes the request to the correct PoP verifier. Two paths exist today:
//! - `cnf.x5t#S256` (RFC 8705 mTLS cert-bound) ⇒ the `cert_bound` verification core; and
//! - `cnf.jkt` (RFC 9449 DPoP-bound) ⇒ `PopRoute::Dpop`, meaning "the caller runs the **full**,
//!   unchanged DPoP verify" — the fast path never bypasses DPoP.
//!
//! ## What this module is NOT (the two seams left open, by design)
//! T1a is transport- and verifier-agnostic so it is unit-testable in isolation. Two inputs are
//! *injected*; wiring each is a tracked follow-up:
//!
//! 1. **The token's `Confirmation`** — populated by the OIDC verifier. The pinned
//!    `solid-oidc-verifier` today exposes only `cnf.jkt` ([`crate::auth::VerifiedToken::cnf_jkt`]);
//!    surfacing `cnf.x5t#S256` as a `Confirmation` is the owner-gated verifier follow-up
//!    (design §10 bead 1). Until then no live token can construct `Confirmation::CertBound`.
//! 2. **The presented certificate thumbprint** — read once per connection by the rustls
//!    optional-client-cert acceptor (T1b, design §10 bead 2 / suite-tracker `ad4o`).
//!    `cert_bound::CertThumbprint::from_cert_der` is the exact entry point that wiring calls.
//!
//! Because both seams are open, this module is **not yet called from
//! [`crate::auth`]**: wiring it in requires *both* the verifier to report a cert-bound confirmation
//! *and* the acceptor to supply the presented cert. Doing half of that would create a fail-open gap
//! (a cert-bound token with no way to see the cert). T1a therefore lands the fail-closed core +
//! exhaustive tests + this documented seam, and the auth-middleware dispatch arm lands with T1b.
//!
//! ## No-downgrade invariants (tested in this module + `cert_bound`)
//! - A `Confirmation::Dpop` token ALWAYS routes to `PopRoute::Dpop`, regardless of any presented
//!   certificate — a certificate can never satisfy a DPoP-bound token, and DPoP verification is not
//!   skipped.
//! - A `Confirmation::CertBound` token NEVER routes to the DPoP path — a cert-bound token cannot be
//!   accepted via the weaker DPoP path (nor bare).
//! - When a token carries BOTH `cnf.jkt` and `cnf.x5t#S256`, `Confirmation::select` resolves to
//!   `Confirmation::MultipleBindings`, which `dispatch` **rejects** (`PopRoute::MultipleBindings`).
//!   The two are *independent* bindings; accepting via only one would bypass the other. Combined
//!   (both-must-hold) verification is a verifier-side follow-up (design §10 bead 1); until it exists,
//!   a multi-binding token is refused rather than partially satisfied — the fail-closed choice.

pub mod cert_bound;
pub mod conn;
pub mod sk;

use cert_bound::{verify_cert_binding, CertBindingOutcome, CertThumbprint};

/// The proof-of-possession confirmation method a verified access token is bound to (its `cnf` claim).
///
/// Constructed by the OIDC verifier from the token's `cnf` member (seam 1 above). Kept here — rather
/// than in the verifier crate — for T1a so the dispatch is testable before the owner-gated verifier
/// change lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmation {
    /// RFC 9449 DPoP-bound: the token carries `cnf.jkt` (the JWK SHA-256 thumbprint). The full DPoP
    /// proof verification (fresh proof signature, `htm`/`htu`/`iat`/`ath`, `jti` replay,
    /// `cnf.jkt == jkt(proof)`) is authoritative for this method.
    Dpop { jkt: String },
    /// RFC 8705 mTLS cert-bound: the token carries `cnf.x5t#S256` (the SHA-256 thumbprint of the
    /// client certificate). Possession is proven in the TLS handshake; the per-request check is the
    /// thumbprint compare in [`cert_bound`].
    CertBound { x5t_s256: CertThumbprint },
    /// The token declares BOTH `cnf.jkt` and `cnf.x5t#S256`. These are independent bindings; combined
    /// (both-must-hold) verification is not implemented in T1a, so such a token is **rejected**
    /// fail-closed rather than accepted by satisfying only one binding (which would bypass the other).
    /// Enabling both is a verifier-side follow-up (design §10 bead 1).
    MultipleBindings,
    /// The token carries no recognised PoP confirmation claim — a bare bearer token. Solid-OIDC
    /// requires sender-constrained (DPoP) tokens, so the caller's `require_dpop` policy rejects this;
    /// modelled explicitly so the dispatch is total and fail-closed rather than defaulting.
    Unbound,
}

impl Confirmation {
    /// Resolve a token's confirmation method from the (already-parsed) `cnf` members, fail-closed:
    /// - exactly one of `cnf.x5t#S256` / `cnf.jkt` ⇒ that method;
    /// - **both** present ⇒ [`Confirmation::MultipleBindings`] (rejected by [`dispatch`]) — the two are
    ///   independent bindings and satisfying only one would bypass the other; combined verification is
    ///   a follow-up;
    /// - neither ⇒ [`Confirmation::Unbound`].
    pub fn select(jkt: Option<String>, x5t_s256: Option<CertThumbprint>) -> Self {
        match (x5t_s256, jkt) {
            (Some(x5t_s256), None) => Confirmation::CertBound { x5t_s256 },
            (None, Some(jkt)) => Confirmation::Dpop { jkt },
            (Some(_), Some(_)) => Confirmation::MultipleBindings,
            (None, None) => Confirmation::Unbound,
        }
    }
}

/// The routing decision [`dispatch`] returns for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopRoute {
    /// Run the existing, **full** RFC 9449 DPoP verification. The fast path adds nothing and skips
    /// nothing on this route — it is the mandatory baseline.
    Dpop,
    /// The token is RFC 8705 cert-bound; the binding was checked with this outcome. On
    /// [`CertBindingOutcome::Confirmed`] the caller injects the verified token; on any
    /// [`CertBindingOutcome::Denied`] the caller returns a 401 (never bare acceptance).
    CertBound(CertBindingOutcome),
    /// The token declares multiple confirmation methods; reject (combined verification unimplemented —
    /// see [`Confirmation::MultipleBindings`]). The caller returns a 401, never a partial acceptance.
    MultipleBindings,
    /// The token carries no PoP binding — the caller's DPoP-required policy rejects it (fail-closed).
    Unbound,
}

/// Route a request to the correct proof-of-possession verifier based on the token's [`Confirmation`].
///
/// `presented_cert` is the thumbprint of the client certificate on the live TLS connection (T1b
/// seam), or `None` when the connection presented no client certificate. It is consulted ONLY for
/// the [`Confirmation::CertBound`] path — a DPoP or unbound token ignores it entirely (a certificate
/// can never satisfy a non-cert-bound token).
pub fn dispatch(confirmation: &Confirmation, presented_cert: Option<&CertThumbprint>) -> PopRoute {
    match confirmation {
        // DPoP is authoritative and unchanged — never satisfied by a certificate, never bypassed.
        Confirmation::Dpop { .. } => PopRoute::Dpop,
        // The RFC 8705 §3 token-binding check (fail-closed inside `verify_cert_binding`).
        Confirmation::CertBound { x5t_s256 } => {
            PopRoute::CertBound(verify_cert_binding(x5t_s256, presented_cert))
        }
        // A token declaring both bindings is refused rather than satisfied by only one (no bypass).
        Confirmation::MultipleBindings => PopRoute::MultipleBindings,
        Confirmation::Unbound => PopRoute::Unbound,
    }
}

#[cfg(test)]
mod tests {
    use super::cert_bound::{CertBindingDenial, CertThumbprint};
    use super::*;

    const CERT_A: &[u8] = b"-----fake-der-cert-A-----";
    const CERT_B: &[u8] = b"-----fake-der-cert-B-----";

    fn cert_bound(cert: &[u8]) -> Confirmation {
        Confirmation::CertBound {
            x5t_s256: CertThumbprint::from_cert_der(cert),
        }
    }

    #[test]
    fn dpop_token_always_routes_to_full_dpop_verify() {
        let conf = Confirmation::Dpop {
            jkt: "thumb".to_string(),
        };
        // No cert presented ⇒ still the DPoP path (not "unbound", not an error).
        assert_eq!(dispatch(&conf, None), PopRoute::Dpop);
        // Even if a certificate IS presented, a DPoP token is NEVER satisfied by it (no cross-method
        // acceptance) — it still goes through the full DPoP verify.
        let presented = CertThumbprint::from_cert_der(CERT_A);
        assert_eq!(dispatch(&conf, Some(&presented)), PopRoute::Dpop);
    }

    #[test]
    fn cert_bound_token_confirmed_on_matching_cert() {
        let conf = cert_bound(CERT_A);
        let presented = CertThumbprint::from_cert_der(CERT_A);
        assert_eq!(
            dispatch(&conf, Some(&presented)),
            PopRoute::CertBound(CertBindingOutcome::Confirmed)
        );
    }

    #[test]
    fn cert_bound_token_denied_when_no_cert_presented() {
        // Fail-closed: cert-bound token, no client cert on the connection ⇒ deny.
        let conf = cert_bound(CERT_A);
        assert_eq!(
            dispatch(&conf, None),
            PopRoute::CertBound(CertBindingOutcome::Denied(
                CertBindingDenial::NoCertPresented
            ))
        );
    }

    #[test]
    fn cert_bound_token_denied_on_wrong_cert() {
        let conf = cert_bound(CERT_A);
        let presented = CertThumbprint::from_cert_der(CERT_B);
        assert_eq!(
            dispatch(&conf, Some(&presented)),
            PopRoute::CertBound(CertBindingOutcome::Denied(
                CertBindingDenial::ThumbprintMismatch
            ))
        );
    }

    #[test]
    fn cert_bound_token_never_accepted_via_dpop_path() {
        // No-downgrade the other direction: a cert-bound confirmation NEVER yields PopRoute::Dpop,
        // for any presented-cert input. It can only ever be Confirmed (matching cert) or Denied.
        let conf = cert_bound(CERT_A);
        for presented in [
            None,
            Some(CertThumbprint::from_cert_der(CERT_A)),
            Some(CertThumbprint::from_cert_der(CERT_B)),
        ] {
            let route = dispatch(&conf, presented.as_ref());
            assert!(
                matches!(route, PopRoute::CertBound(_)),
                "cert-bound token must never route to the DPoP or Unbound path, got {route:?}"
            );
        }
    }

    #[test]
    fn unbound_token_routes_to_unbound() {
        assert_eq!(dispatch(&Confirmation::Unbound, None), PopRoute::Unbound);
    }

    #[test]
    fn select_rejects_multiple_bindings_no_bypass() {
        // A token carrying BOTH cnf.jkt and cnf.x5t#S256 must NOT be accepted by satisfying only one
        // binding (that would bypass the other). It resolves to MultipleBindings and dispatch rejects.
        let x5t = CertThumbprint::from_cert_der(CERT_A);
        let conf = Confirmation::select(Some("jkt-thumb".to_string()), Some(x5t));
        assert_eq!(conf, Confirmation::MultipleBindings);
        // Even presenting the matching certificate must not make it acceptable via the cert path.
        let presented = CertThumbprint::from_cert_der(CERT_A);
        assert_eq!(
            dispatch(&conf, Some(&presented)),
            PopRoute::MultipleBindings
        );
        assert_eq!(dispatch(&conf, None), PopRoute::MultipleBindings);
    }

    #[test]
    fn select_maps_jkt_only_to_dpop_and_none_to_unbound() {
        assert_eq!(
            Confirmation::select(Some("jkt".to_string()), None),
            Confirmation::Dpop {
                jkt: "jkt".to_string()
            }
        );
        assert_eq!(Confirmation::select(None, None), Confirmation::Unbound);
    }
}
