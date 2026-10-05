// AUTHORED-BY Claude Opus 4.8
//! Pre-crypto PUBLIC-READ skip (skip-crypto optimization, opt 3 of `research/lws-design-records.md`
//! §5; RSS `decisions/0002`).
//!
//! For a read whose response is **fully identity-independent** — there is no caller identity to
//! resolve (no `Authorization` header) and the effective ACL grants the PUBLIC `acl:Read` — this thin
//! middleware runs the EXISTING anonymous WAC predicate (no crypto) BEFORE the auth layer and serves
//! the read directly, short-circuiting. There is no proof to verify on such a request, so the win is
//! a marginal one (one fewer public-token construction + an earlier WAC pass), NOT a crypto saving.
//!
//! ## The hard scope limit — NO `Authorization`/`DPoP` header (load-bearing, security-critical)
//! The skip fires **only for a GET/HEAD that carries NEITHER an `Authorization` NOR a `DPoP`
//! header.** A request that carries credentials (or a `DPoP` proof header) is NEVER short-circuited —
//! it falls through to the full auth path. For the `Authorization` case there are two independent,
//! decisive reasons, BOTH proven by the WAC-Allow conformance suite
//! (`web-access-control/wac-allow/public-access-{direct,indirect}.feature`), which an earlier
//! "serve any proof-carrying public read as anonymous" attempt FAILED:
//!
//! 1. **`WAC-Allow user=` is identity-DEPENDENT.** The `user` audience advertises what THIS requester
//!    may do. An authenticated OWNER of a public resource holds `read/write/control`, NOT the public
//!    `read`. Serving such a request as anonymous would emit `user="read"` and so UNDER-REPORT the
//!    owner's access — a wrong, observable response. Computing the correct `user=` requires the
//!    VERIFIED WebID, i.e. the crypto. So a credentialed read's response is NOT identity-independent.
//! 2. **A forged proof is INDISTINGUISHABLE from a legitimate owner's proof without the crypto.** To
//!    serve a forged-WebID proof as anonymous (harmless) we would have to serve EVERY proof-carrying
//!    public read as anonymous — including the legitimate owner's — which is exactly the wrong
//!    behaviour in (1). The only thing that tells a forged proof from an owner's is verifying it. So
//!    "skip the crypto for a credentialed public read" cannot be both correct (owner sees full modes)
//!    and safe (forged WebID is ignored): the two collapse without the verify.
//!
//! Therefore opt-3 is scoped to the genuinely-anonymous case. (Why opt-3-for-CREDENTIALED-reads is
//! UNSAFE — and opt 1/2 — is recorded in full in RSS `decisions/0002`, which is NOT in this tree
//! and is NOT reconstructed by `research/lws-design-records.md` §5.) The `DPoP` header is ALSO a
//! fall-through trigger so a no-`Authorization` GET carrying a malformed `DPoP` header keeps the
//! auth path's canonical `400` (which it rejects even without `Authorization`) rather than being
//! served as anonymous here.
//!
//! ## Method scope — GET/HEAD ONLY
//! Even within the no-`Authorization` case it fires for GET/HEAD only; every other verb passes
//! straight through (a mutation must never skip the auth path — defence in depth; an anonymous
//! mutation is denied there anyway).
//!
//! ## Dispatch — delegate to `serve_read` with a public token (ONE WAC pass)
//! For a no-credential GET/HEAD, the middleware constructs `token = VerifiedToken::public()` and
//! delegates STRAIGHT to the SAME `serve_read` the handler uses — it does NOT run its own separate
//! WAC predicate first. `serve_read` does exactly ONE effective-ACL resolution (web_id = None — the
//! anonymous decision) and returns: a PUBLIC read → 200 + body; an anonymous denial → the same 401 +
//! `WWW-Authenticate` the full path returns; a malformed target → the canonical 400 (it calls
//! `parse_target` itself). So the response is byte-identical to the full anonymous path (it IS the same
//! call), with the SAME single ACL resolution — the middleware only saves the auth middleware's
//! public-token construction + `AuthRequest` build + a layer hop. (Delegating avoids the double WAC
//! pass an earlier pre-check-then-serve_read version did — a review-flagged regression.)
//!
//! ## Security invariants (tested in `tests/public_read_skip.rs`)
//! - **INV-1 ANONYMOUS-EQUIVALENCE.** When the skip fires, the FULL response tuple
//!   {status, body, ALL headers incl. `WAC-Allow` + `ETag` + `Content-*`} is byte-identical to a
//!   genuinely anonymous request for the same target+Origin — it IS the same `serve_read` call with
//!   the same public token, on a request that carries no credentials.
//! - **INV-2 IDENTITY-INDEPENDENCE.** The skip fires ONLY for a request with no `Authorization`/`DPoP`
//!   header, and serves with `VerifiedToken::public()` (`web_id = None`). It NEVER reads a claimed
//!   WebID (no `peek_claims`); a credentialed request is never short-circuited, so a forged WebID can
//!   never influence a served response — it is rejected by the verifier on the full path.
//! - **INV-3 NO-ORACLE.** `WAC-Allow` carries `user == public` on the skip (correct: the caller IS the
//!   public); existence is not distinguished (a missing publicly-readable resource is the same 404 the
//!   anonymous path produces).
//! - **INV-6 ORIGIN-FAIL-CLOSED.** `serve_read` reads the origin via the handler's own
//!   `request_origin` and resolves the anonymous ACL with it, so an `acl:origin`-scoped public grant
//!   is served ONLY for a matching Origin; a no-Origin / non-matching anonymous caller gets the same
//!   401 the full anonymous path returns (`authorize_read(web_id = None, origin)` is fail-closed).

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::auth::VerifiedToken;
use crate::ldp::handler::{serve_read, LdpState};
use crate::store::Store;

/// The pre-crypto public-read skip middleware. See the module docs for the full contract.
///
/// State is the `Arc<LdpState<S>>` (the store + base URL + ACL cache + notification hub) — the SAME
/// state the LDP handlers carry, so a short-circuited read is served over an identical `serve_read`.
pub async fn public_read_skip_middleware<S>(
    State(state): State<Arc<LdpState<S>>>,
    req: Request,
    next: Next,
) -> Response
where
    S: Store + 'static,
{
    // GET/HEAD ONLY. Every other verb (incl. OPTIONS) goes straight to the unchanged auth path.
    // `with_body` distinguishes the two reads for `serve_read`.
    let with_body = match *req.method() {
        Method::GET => true,
        Method::HEAD => false,
        _ => return next.run(req).await,
    };

    // HARD SCOPE LIMIT (security-critical — see the module docs): the skip fires ONLY for a request
    // that carries NEITHER an `Authorization` NOR a `DPoP` header. A credentialed request is NEVER
    // short-circuited — it falls through to the full verifier — because (1) `WAC-Allow user=` is
    // identity-dependent (an authenticated owner of a public resource must see read/write/control, not
    // the public set), and (2) a forged-WebID proof is indistinguishable from a legitimate owner's
    // proof without the crypto. Only the verifier can resolve both. Presence of EITHER header (not its
    // contents) gates this — we never parse/trust the unverified token.
    //
    // The `DPoP` header is checked too, NOT just `Authorization`: the auth middleware rejects a
    // PRESENT-but-malformed (non-UTF-8) `DPoP` header with a `400` even when `Authorization` is absent
    // (`auth::auth_middleware`). Falling through whenever a `DPoP` header is present preserves that
    // canonical `400` — without this check, a no-`Authorization` GET carrying a malformed `DPoP`
    // header would be served as anonymous here instead of 400, a behaviour divergence from the full
    // path. A `DPoP` header is only meaningful alongside `Authorization` anyway, so falling through
    // costs nothing on the legitimate anonymous path.
    let headers = req.headers();
    if headers.contains_key(header::AUTHORIZATION)
        || headers.contains_key(axum::http::HeaderName::from_static("dpop"))
    {
        return next.run(req).await;
    }

    // SINGLE WAC PASS (no double resolution): for a no-credential request, the auth middleware would
    // construct a public token and call the SAME `serve_read`, which does exactly ONE WAC pass and
    // returns Allow→200 / Unauthenticated→401 / store-error. So we DON'T run a separate pre-check WAC
    // pass (which would resolve the ACL twice — the regression review flagged); we delegate STRAIGHT to
    // `serve_read` with the public token. The result is byte-identical to the full anonymous path (it
    // IS the same call), with the SAME single ACL resolution the full path does — the middleware only
    // saves the auth-middleware's public-token construction + `AuthRequest` build + a layer hop.
    //
    // A malformed TARGET still becomes the canonical `400` because `serve_read` calls `parse_target`
    // internally and propagates the `ServerError::BadRequest` (rendered here via `into_response`),
    // identical to the full path. The public token carries no WebID (INV-2); `serve_read` emits
    // `WAC-Allow` with `user == public`, correct because the caller IS the public (INV-3); a private
    // resource yields the same 401 + `WWW-Authenticate` the full anonymous path returns (INV-1, and no
    // 401-vs-403 oracle — an anonymous denial is always 401).
    let token = VerifiedToken::public();
    let uri = req.uri().clone();
    // Clone the headers so `serve_read` sees EXACTLY the request headers (Accept / Range / Origin …)
    // the full anonymous path would carry — INV-1.
    let headers = req.headers().clone();
    match serve_read::<S>(&state, &token, &uri, &headers, with_body).await {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}
