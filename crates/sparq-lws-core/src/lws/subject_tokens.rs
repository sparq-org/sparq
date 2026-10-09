//! Subject token verification for the LWS authorization server (LWS 1.0 core section 5.2.3,
//! "validate before issuing"). Each authentication suite is checked the way its specification and
//! the Touchstone reference authorization server check it:
//!
//! - **did:key** (`lws10-authn-ssi-did-key`): a self-issued JWT whose subject is a did:key. The key
//!   comes from the identifier itself (P-256, multicodec `0x1200`, signed ES256; or Ed25519,
//!   multicodec `0xed`, signed EdDSA), and `kid` must name the one verification method of the DID
//!   document the identifier derives (`did:key:z…#z…`, or just its fragment).
//! - **Controlled identifier** (`lws10-authn-ssi-cid`): a self-issued JWT whose subject is an HTTPS
//!   URI. The controlled identifier document is fetched from the subject, its `id` must be the
//!   subject, and `kid` must name a method of its `authentication` relationship (embedded or
//!   referenced) that the subject controls and that is neither revoked nor expired.
//! - **OpenID Connect** (`lws10-authn-openid`): an ID Token. The subject's identity document must
//!   name the token's issuer, as an `lws:OpenIdProvider` service or (Solid-OIDC, as the Community
//!   Solid Server accepts) with `solid:oidcIssuer`; the issuer's key comes from OpenID Connect
//!   Discovery; `azp` names the client. A Solid-OIDC ID Token (one addressed to `solid`, or bound
//!   to a key by `cnf.jkt`) is exchanged only with a DPoP proof of that key on the token request
//!   (RFC 9449 section 4.3, see [`check_dpop`]): a copied ID Token is worth nothing without it.
//!
//! Self-issued credentials need `sub = iss = client_id`, an `aud` that includes this authorization
//! server, an `exp` in the future and an `iat` not ahead of now. `alg: none` is always refused.
//!
//! Documents are fetched with the server's HTTP client. Unless `allow_insecure_fetch` is set, only
//! `https:` URLs are fetched, and never from loopback, private, link-local or otherwise non-global
//! addresses: every redirect hop is checked before it is requested (the client follows none
//! itself), the client's resolver hands out public addresses only, so the connection goes where
//! the check looked, and no proxy is used.

use std::collections::HashMap;
use std::net::IpAddr;

use serde_json::Value;

use super::jose::{self, Jws};
use super::{has_type, is_uri, LwsConfig, LWS_NS};

pub const JWT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:jwt";
pub const ID_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";

/// Allowed clock skew, in seconds, for subject token times (the reference's).
pub const SKEW_SECS: i64 = 60;

/// Largest identity document, discovery document or JWKS read.
const MAX_DOC: usize = 1024 * 1024;

const SOLID_OIDC_ISSUER: &str = "http://www.w3.org/ns/solid/terms#oidcIssuer";
const CID_SERVICE: &str = "https://www.w3.org/ns/cid/v1#service";
const CID_SERVICE_ENDPOINT: &str = "https://www.w3.org/ns/cid/v1#serviceEndpoint";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// How long a DPoP proof's `iat` may lie from now, in seconds.
pub const DPOP_WINDOW_SECS: i64 = 60;

/// Most bytes the DPoP replay cache holds at once, counted by [`entry_bytes`]; past it (after the
/// expired entries are dropped) a new proof is refused rather than remembered, so replay
/// protection never silently lapses.
const MAX_DPOP_BYTES: usize = 16 << 20;

/// How many of an OpenID Provider's keys an ID Token's signature is tried against.
const MAX_UNNAMED_KEYS: usize = 8;

/// The replay cache is split so that no party can use up room another needs. Half of it is held
/// for the providers named in `SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS`; every other provider
/// shares the other half, each held to [`MAX_DPOP_BYTES_PER_OPEN_ISSUER`]. Within either pool each
/// identity (issuer and subject) and each key is held to its own share, so an identity, however
/// many keys it makes, crowds out only itself, and a provider that registers identities freely
/// crowds out only providers outside the trusted list. Every share is in bytes, so long names
/// fill a share sooner rather than holding more than it.
const MAX_DPOP_BYTES_TRUSTED: usize = MAX_DPOP_BYTES / 2;
const MAX_DPOP_BYTES_OPEN: usize = MAX_DPOP_BYTES - MAX_DPOP_BYTES_TRUSTED;
/// How many bytes of live entries all the identities of one provider outside the trusted list
/// may hold.
const MAX_DPOP_BYTES_PER_OPEN_ISSUER: usize = 1 << 20;
/// How many bytes of live entries one identity may hold: a client presents a proof per token
/// request, and an ordinary one fits a couple of dozen entries in one window.
const MAX_DPOP_BYTES_PER_IDENTITY: usize = 64 << 10;
/// How many bytes of live entries one key may hold.
const MAX_DPOP_BYTES_PER_KEY: usize = 64 << 10;
/// The most slots each of the replay cache's tables keeps per live entry it holds, beside
/// [`DPOP_SPARE_SLOTS`]: a table grows to at most about twice what it holds, and one left more than
/// this much larger as entries expire is shrunk (see [`settle`]).
const DPOP_SLOTS_PER_ENTRY: usize = 4;
/// The slots each table may keep beside those its live entries are charged for.
const DPOP_SPARE_SLOTS: usize = 16;
/// What one slot of each table costs: the entry by key and `jti` (with a hash table's control byte,
/// and its unused eighth), a share count, and a place in expiry order.
const SEEN_SLOT: usize =
    (std::mem::size_of::<((String, String), (Vec<Share>, usize))>() + 1) * 8 / 7 + 1;
const USED_SLOT: usize = (std::mem::size_of::<(Share, usize)>() + 1) * 8 / 7 + 1;
const EXPIRY_SLOT: usize = std::mem::size_of::<Expiry>();
/// The most shares an entry counts against (a pool, an identity, a key, an issuer).
const MAX_SHARES: usize = 4;
/// What a replay cache entry costs beside its strings: the table slots it may keep allocated,
/// which outlast it until the tables settle, and the string headers it holds. Charging the
/// tables' capacity to the entries keeps every byte the cache holds within its shares.
const DPOP_ENTRY_OVERHEAD: usize =
    DPOP_SLOTS_PER_ENTRY * (SEEN_SLOT + EXPIRY_SLOT + MAX_SHARES * USED_SLOT) + 64;
/// What a share costs beside its strings.
const DPOP_SHARE_OVERHEAD: usize = std::mem::size_of::<Share>();

/// Whose proof a replay cache entry is: the verified ID Token's issuer and subject, and whether
/// the issuer is trusted. Entries are still keyed by the proof key and `jti` alone, so a proof
/// replayed under another identity is refused too.
pub struct ProofOwner<'a> {
    pub issuer: &'a str,
    pub subject: &'a str,
    pub trusted: bool,
}

/// One quota the replay cache keeps: a pool, an issuer, an identity or a key.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Share {
    Pool(bool),
    Issuer(String),
    Identity(String, String),
    Key(String),
}

impl Share {
    fn limit(&self) -> usize {
        match self {
            Share::Pool(true) => MAX_DPOP_BYTES_TRUSTED,
            Share::Pool(false) => MAX_DPOP_BYTES_OPEN,
            Share::Issuer(_) => MAX_DPOP_BYTES_PER_OPEN_ISSUER,
            Share::Identity(..) => MAX_DPOP_BYTES_PER_IDENTITY,
            Share::Key(_) => MAX_DPOP_BYTES_PER_KEY,
        }
    }

    fn bytes(&self) -> usize {
        DPOP_SHARE_OVERHEAD
            + match self {
                Share::Pool(_) => 0,
                Share::Issuer(s) | Share::Key(s) => s.len(),
                Share::Identity(i, s) => i.len() + s.len(),
            }
    }
}

/// What an entry holds: its key and `jti` twice (its map key and its place in expiry order), and
/// its shares twice (its own list, and at most once more as a key of the share counts).
fn entry_bytes(shares: &[Share], jkt: &str, jti: &str) -> usize {
    DPOP_ENTRY_OVERHEAD
        + 2 * (jkt.len() + jti.len())
        + 2 * shares.iter().map(Share::bytes).sum::<usize>()
}

/// The DPoP proof ids (`jti`) seen at the token endpoint, by the key that signed them, until
/// their proofs are too old to be accepted anyway. Each entry counts against every share it
/// belongs to, by the bytes it holds (see [`MAX_DPOP_BYTES_TRUSTED`]). Expired entries leave in expiry order, a few at a
/// time as new ones arrive, so no request scans the whole cache.
#[derive(Default)]
pub struct DpopReplay(std::sync::Mutex<Replay>);

/// An entry's place in expiry order: when it expires, and its key and `jti`.
type Expiry = std::cmp::Reverse<(i64, String, String)>;

/// Rebuild a table left much larger than what it holds, so the slots it keeps stay within what
/// its live entries are charged for ([`DPOP_SLOTS_PER_ENTRY`]). The live entries move into a new
/// table sized for them and the old allocation is freed whole: a shrink in place may keep it.
fn settle<K: Eq + std::hash::Hash, V>(table: &mut std::collections::HashMap<K, V>) {
    if table.capacity() > DPOP_SLOTS_PER_ENTRY * table.len() + DPOP_SPARE_SLOTS {
        let mut fresh = std::collections::HashMap::with_capacity(table.len());
        fresh.extend(table.drain());
        *table = fresh;
    }
}

/// As [`settle`], for the expiry order.
fn settle_heap<T: Ord>(heap: &mut std::collections::BinaryHeap<T>) {
    if heap.capacity() > DPOP_SLOTS_PER_ENTRY * heap.len() + DPOP_SPARE_SLOTS {
        let mut fresh = Vec::with_capacity(heap.len());
        fresh.extend(std::mem::take(heap).into_vec());
        *heap = std::collections::BinaryHeap::from(fresh);
    }
}

#[derive(Default)]
struct Replay {
    /// Each live entry, by key and `jti`, with the shares it counts against and its bytes.
    seen: std::collections::HashMap<(String, String), (Vec<Share>, usize)>,
    /// The bytes each share's live entries hold.
    used: std::collections::HashMap<Share, usize>,
    expiry: std::collections::BinaryHeap<Expiry>,
}

impl DpopReplay {
    /// Remember `jti` from the key `jkt`, presented by `owner`, until `until`; `false` when it was
    /// seen already (a replay), or one of its shares is full. Fails closed: an entry is never
    /// evicted before it expires.
    fn first_use(
        &self,
        owner: &ProofOwner<'_>,
        jkt: &str,
        jti: &str,
        until: i64,
        now: i64,
    ) -> bool {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Replay { seen, used, expiry } = &mut *guard;
        while let Some(std::cmp::Reverse((t, _, _))) = expiry.peek() {
            if *t > now {
                break;
            }
            let Some(std::cmp::Reverse((_, k, j))) = expiry.pop() else {
                break;
            };
            let (shares, bytes) = seen.remove(&(k, j)).unwrap_or_default();
            for share in shares {
                if let Some(n) = used.get_mut(&share) {
                    *n -= bytes;
                    if *n == 0 {
                        used.remove(&share);
                    }
                }
            }
        }
        settle(seen);
        settle(used);
        settle_heap(expiry);
        let entry = (jkt.to_string(), jti.to_string());
        if seen.contains_key(&entry) {
            return false;
        }
        let mut shares = vec![
            Share::Pool(owner.trusted),
            Share::Identity(owner.issuer.to_string(), owner.subject.to_string()),
            Share::Key(jkt.to_string()),
        ];
        if !owner.trusted {
            shares.push(Share::Issuer(owner.issuer.to_string()));
        }
        let bytes = entry_bytes(&shares, jkt, jti);
        if shares
            .iter()
            .any(|s| used.get(s).copied().unwrap_or(0) + bytes > s.limit())
        {
            return false;
        }
        for share in &shares {
            *used.entry(share.clone()).or_default() += bytes;
        }
        seen.insert(entry, (shares, bytes));
        expiry.push(std::cmp::Reverse((until, jkt.to_string(), jti.to_string())));
        true
    }
}

/// What the token request carries for proof of possession: its `DPoP` header and the replay
/// store the proofs' ids go into.
pub struct DpopContext<'a> {
    pub proof: Option<&'a str>,
    pub replay: &'a DpopReplay,
}

/// The `cnf.jkt` a token is bound to, if any.
fn bound_jkt(jws: &Jws) -> Option<&str> {
    jws.claims
        .get("cnf")
        .and_then(|c| c.get("jkt"))
        .and_then(Value::as_str)
}

/// RFC 9449 section 4.3: `proof` is a DPoP proof JWT (`typ` `dpop+jwt`, an asymmetric `alg`, a
/// public `jwk` it verifies with) for a POST to `htu`, issued within [`DPOP_WINDOW_SECS`] of `now`,
/// with a `jti` not seen before, by the key whose RFC 7638 thumbprint is `jkt`.
pub fn check_dpop(
    proof: Option<&str>,
    htu: &str,
    jkt: &str,
    now: i64,
    replay: &DpopReplay,
    owner: &ProofOwner<'_>,
) -> Result<(), String> {
    let proof = proof.ok_or("a DPoP-bound ID Token needs a DPoP proof on the token request")?;
    let jws = Jws::parse(proof).ok_or("the DPoP proof is not a JWT")?;
    if jws.typ() != Some("dpop+jwt") {
        return Err("the DPoP proof's typ is not dpop+jwt".into());
    }
    if !matches!(jws.alg(), Some("ES256" | "RS256" | "EdDSA")) {
        return Err("the DPoP proof's alg is not a supported asymmetric one".into());
    }
    let jwk = jws
        .header
        .get("jwk")
        .filter(|k| k.is_object())
        .ok_or("the DPoP proof carries no jwk")?;
    if ["d", "p", "q", "dp", "dq", "qi", "k"]
        .iter()
        .any(|m| jwk.get(m).is_some())
    {
        return Err("the DPoP proof's jwk is not a public key".into());
    }
    if !jws.verify_jwk(jwk) {
        return Err("the DPoP proof's signature does not verify".into());
    }
    // The key's own thumbprint, not the JWK's spelling of it: the replay cache's quotas are
    // keyed on it, so another spelling of one key is not another key.
    if jose::key_thumbprint(jwk).as_deref() != Some(jkt) {
        return Err("the DPoP proof is not signed by the key the ID Token is bound to".into());
    }
    if jws.claim_str("htm") != Some("POST") {
        return Err("the DPoP proof's htm is not POST".into());
    }
    let same_target = |a: &str, b: &str| {
        let strip = |u: &str| {
            url::Url::parse(u).ok().map(|mut u| {
                u.set_query(None);
                u.set_fragment(None);
                u
            })
        };
        strip(a).is_some_and(|a| Some(a) == strip(b))
    };
    if !jws.claim_str("htu").is_some_and(|u| same_target(u, htu)) {
        return Err("the DPoP proof's htu is not the token endpoint".into());
    }
    let iat = jws.claim_time("iat")?.ok_or("the DPoP proof has no iat")?;
    if (iat - now as f64).abs() > DPOP_WINDOW_SECS as f64 {
        return Err("the DPoP proof is not fresh".into());
    }
    let jti = jws
        .claim_str("jti")
        .filter(|j| !j.is_empty() && j.len() <= 256)
        .ok_or("the DPoP proof has no jti")?;
    if !replay.first_use(
        owner,
        jkt,
        jti,
        // Kept until the proof could no longer be fresh, a part second rounded up.
        (iat.ceil() as i64).saturating_add(DPOP_WINDOW_SECS + 1),
        now,
    ) {
        return Err("the DPoP proof was used before".into());
    }
    Ok(())
}

/// Who a valid subject token authenticates, and the client presenting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub subject: String,
    pub client: String,
}

/// Verify a JWT-shaped subject token of `token_type` for an exchange at this authorization server.
/// The error is the reason, for `error_description`.
pub async fn verify(
    cfg: &LwsConfig,
    http: &reqwest::Client,
    token: &str,
    token_type: &str,
    dpop: &DpopContext<'_>,
) -> Result<Verified, String> {
    match token_type {
        JWT_TOKEN_TYPE => {
            let jws = parse(token)?;
            if jws
                .claim_str("sub")
                .is_some_and(|s| s.starts_with("did:key:"))
            {
                did_key(cfg, &jws)
            } else {
                cid(cfg, http, &jws).await
            }
        }
        ID_TOKEN_TYPE => oidc(cfg, http, &parse(token)?, dpop).await,
        other => Err(format!("unsupported subject_token_type {other}")),
    }
}

/// A signed JWT: three parts, JSON header and claims, and an `alg` that is not `none`.
pub fn parse(token: &str) -> Result<Jws, String> {
    let jws = Jws::parse(token).ok_or("the subject token is not a compact JWS")?;
    match jws.alg() {
        None => Err("the subject token names no alg".into()),
        Some(a) if a.eq_ignore_ascii_case("none") => Err("alg none is refused".into()),
        Some(_) => Ok(jws),
    }
}

/// `exp` present and not past, `iat` present and not ahead of now, `nbf` (when present) reached;
/// each, when present, a time [`Jws::claim_time`] reads.
pub fn check_times(jws: &Jws, now: i64) -> Result<(), String> {
    let exp = jws.claim_time("exp")?.ok_or("exp is required")?;
    if exp <= now.saturating_sub(SKEW_SECS) as f64 {
        return Err("the credential has expired".into());
    }
    let iat = jws.claim_time("iat")?.ok_or("iat is required")?;
    if iat > now.saturating_add(SKEW_SECS) as f64 {
        return Err("iat lies in the future".into());
    }
    if jws
        .claim_time("nbf")?
        .is_some_and(|nbf| nbf > now.saturating_add(SKEW_SECS) as f64)
    {
        return Err("the credential is not valid yet".into());
    }
    Ok(())
}

/// Whether `aud` names this authorization server (its issuer identifier, with or without the
/// trailing slash).
fn addressed_to_us(cfg: &LwsConfig, jws: &Jws) -> bool {
    let issuer = cfg.issuer();
    jws.audiences()
        .iter()
        .any(|a| a == issuer || a.strip_suffix('/') == Some(issuer))
}

/// The self-issued credential checks shared by did:key and CID subjects; the subject on success.
pub fn self_issued(cfg: &LwsConfig, jws: &Jws, now: i64) -> Result<String, String> {
    let sub = jws.claim_str("sub").filter(|s| !s.is_empty());
    let Some(sub) = sub else {
        return Err("sub, iss and client_id must be the same identifier (sub is missing)".into());
    };
    if jws.claim_str("iss") != Some(sub) || jws.claim_str("client_id") != Some(sub) {
        return Err("sub, iss and client_id must be the same identifier".into());
    }
    check_times(jws, now)?;
    if !addressed_to_us(cfg, jws) {
        return Err("aud does not include this authorization server".into());
    }
    Ok(sub.to_string())
}

// ---------------------------------------------------------------- did:key

/// A public key a did:key or a multibase verification method carries.
#[derive(Debug, Clone)]
pub enum MultikeyPublic {
    P256(p256::PublicKey),
    Ed25519([u8; 32]),
}

impl MultikeyPublic {
    /// Verify `jws` with this key, insisting on the algorithm the key type implies.
    pub fn verify(&self, jws: &Jws) -> Result<(), String> {
        let ok = match self {
            MultikeyPublic::P256(k) => {
                if jws.alg() != Some("ES256") {
                    return Err(format!(
                        "a P-256 key signs ES256, not {}",
                        jws.alg().unwrap_or_default()
                    ));
                }
                jws.verify_es256(k)
            }
            MultikeyPublic::Ed25519(k) => {
                if !matches!(jws.alg(), Some("EdDSA" | "Ed25519")) {
                    return Err(format!(
                        "an Ed25519 key signs EdDSA, not {}",
                        jws.alg().unwrap_or_default()
                    ));
                }
                jws.verify_ed25519(k)
            }
        };
        if ok {
            Ok(())
        } else {
            Err("the signature does not verify".into())
        }
    }
}

/// The longest base58btc multikey [`decode_multikey`] considers, in characters: well past the 48 of
/// the largest supported key (a compressed P-256 point with its 2-byte multicodec prefix).
pub const MAX_MULTIKEY_CHARS: usize = 128;

/// Decode a multibase base58btc (`z…`) multikey: P-256 (multicodec `0x1200`, the compressed SEC1
/// point) or Ed25519 (multicodec `0xed`, the raw 32-byte key).
///
/// The supported keys are at most 35 bytes (48 base58 characters), so a longer value is refused
/// before it is decoded (base58 decoding is quadratic in the input), and the decode goes into a
/// fixed buffer that a longer value overflows.
pub fn decode_multikey(multibase: &str) -> Result<MultikeyPublic, String> {
    let b58 = multibase
        .strip_prefix('z')
        .ok_or("not a base58btc multibase value")?;
    if b58.len() > MAX_MULTIKEY_CHARS {
        return Err("the multikey is longer than any supported key".into());
    }
    let mut buf = [0u8; 64];
    let len = bs58::decode(b58)
        .onto(&mut buf)
        .map_err(|_| "not valid base58btc, or longer than any supported key")?;
    match &buf[..len] {
        [0x80, 0x24, point @ ..] if point.len() == 33 && matches!(point[0], 2 | 3) => {
            jose::ec_public_from_sec1(point)
                .map(MultikeyPublic::P256)
                .ok_or_else(|| "the point is not on P-256".into())
        }
        [0xed, 0x01, raw @ ..] if raw.len() == 32 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(raw);
            Ok(MultikeyPublic::Ed25519(k))
        }
        _ => Err("the key is neither a compressed P-256 nor an Ed25519 multikey".into()),
    }
}

/// The key a `did:key:z…` identifier names.
pub fn did_key_public(did: &str) -> Result<MultikeyPublic, String> {
    let mb = did.strip_prefix("did:key:").ok_or("not a did:key")?;
    if mb.contains(['#', '?', '/', ':']) {
        return Err("a did:key subject carries no path, query or fragment".into());
    }
    decode_multikey(mb)
}

/// Whether `kid` names the one verification method of the DID document `did` derives:
/// `did:key:z…#z…`, or its fragment alone (with or without the `#`).
pub fn did_key_kid_matches(did: &str, kid: Option<&str>) -> bool {
    let Some(mb) = did.strip_prefix("did:key:") else {
        return false;
    };
    let method = format!("{did}#{mb}");
    kid.is_some_and(|k| k == method || k == mb || k.strip_prefix('#') == Some(mb))
}

fn did_key(cfg: &LwsConfig, jws: &Jws) -> Result<Verified, String> {
    let did = self_issued(cfg, jws, jose::now_secs())?;
    if !did_key_kid_matches(&did, jws.kid()) {
        return Err(format!(
            "the kid names no verification method of the DID document {did} derives"
        ));
    }
    did_key_public(&did)?.verify(jws)?;
    Ok(Verified {
        subject: did.clone(),
        client: did,
    })
}

// ---------------------------------------------------------------- controlled identifiers

async fn cid(cfg: &LwsConfig, http: &reqwest::Client, jws: &Jws) -> Result<Verified, String> {
    let subject = self_issued(cfg, jws, jose::now_secs())?;
    if !(subject.starts_with("https://") || subject.starts_with("http://")) {
        return Err("a controlled identifier credential needs an https subject".into());
    }
    let kid = jws
        .kid()
        .filter(|k| !k.is_empty())
        .ok_or("a CID credential names its verification method in kid, and this one has none")?;
    let (_, body) = fetch(
        cfg,
        http,
        &subject,
        "application/cid+json, application/ld+json, application/json",
    )
    .await?;
    let doc: Value = serde_json::from_slice(&body)
        .map_err(|_| format!("{subject} is not a JSON controlled identifier document"))?;
    let key = cid_method_key(&doc, &subject, kid, jose::now_secs())?;
    key.verify(jws)?;
    Ok(Verified {
        subject: subject.clone(),
        client: subject,
    })
}

/// A key a CID verification method publishes.
#[derive(Debug, Clone)]
pub enum MethodKey {
    Jwk(Value),
    Multikey(MultikeyPublic),
}

impl MethodKey {
    pub fn verify(&self, jws: &Jws) -> Result<(), String> {
        match self {
            MethodKey::Multikey(k) => k.verify(jws),
            MethodKey::Jwk(jwk) => {
                // The key type fixes the algorithm; verify_jwk refuses any other pairing.
                if jws.verify_jwk(jwk) {
                    Ok(())
                } else {
                    Err("the signature does not verify with the verification method's key".into())
                }
            }
        }
    }
}

/// Resolve a reference in the subject's document: `#frag` against the document URL, anything
/// else as it is.
fn resolve_ref(r: &str, subject: &str) -> String {
    match r.strip_prefix('#') {
        Some(frag) => format!("{}#{frag}", subject.split('#').next().unwrap_or(subject)),
        None => r.to_string(),
    }
}

/// The key of the authentication method `kid` names in the controlled identifier document `doc`
/// of `subject` (CID 1.0 section 3.3): the document must be the subject's, the method must be in
/// the `authentication` relationship (embedded, or referenced and defined under
/// `verificationMethod`), controlled by the subject, and neither revoked nor expired at `now`.
pub fn cid_method_key(
    doc: &Value,
    subject: &str,
    kid: &str,
    now: i64,
) -> Result<MethodKey, String> {
    if doc.get("id").and_then(Value::as_str) != Some(subject) {
        return Err(format!(
            "the controlled identifier document at {subject} names another id"
        ));
    }
    let method_id = if kid.contains(':') {
        kid.to_string()
    } else {
        resolve_ref(&format!("#{}", kid.trim_start_matches('#')), subject)
    };
    let id_of = |m: &Value| {
        m.get("id")
            .and_then(Value::as_str)
            .map(|i| resolve_ref(i, subject))
    };
    let as_list = |v: Option<&Value>| -> Vec<Value> {
        match v {
            Some(Value::Array(a)) => a.clone(),
            Some(v @ (Value::Object(_) | Value::String(_))) => vec![v.clone()],
            _ => Vec::new(),
        }
    };
    // Only `method_id` is looked for: the method it names, defined once, and a reference to it
    // (or an embedded method with its id) in `authentication`. Each list is read once, however
    // long a document makes them.
    let named = |m: &Value| m.is_object() && id_of(m).as_deref() == Some(method_id.as_str());
    let defined = as_list(doc.get("verificationMethod"))
        .into_iter()
        .find(|c| named(c));
    let mut method = None;
    for entry in as_list(doc.get("authentication")) {
        match &entry {
            Value::String(r) if defined.is_some() && resolve_ref(r, subject) == method_id => {
                method = defined;
                break;
            }
            Value::Object(_) if named(&entry) => {
                method = Some(entry);
                break;
            }
            _ => {}
        }
    }
    let method = method.ok_or_else(|| {
        format!("the controlled identifier document names no authentication method {method_id}")
    })?;
    if method
        .get("controller")
        .and_then(Value::as_str)
        .map(|c| resolve_ref(c, subject))
        .as_deref()
        != Some(subject)
    {
        return Err(format!(
            "verification method {method_id} is controlled by another identifier"
        ));
    }
    for bound in ["revoked", "expires"] {
        if let Some(v) = method.get(bound).filter(|v| !v.is_null()) {
            let at = v.as_str().and_then(parse_datetime).ok_or_else(|| {
                format!("verification method {method_id} has an unreadable {bound} time")
            })?;
            if now >= at {
                return Err(format!("verification method {method_id} is {bound}"));
            }
        }
    }
    if let Some(jwk) = method.get("publicKeyJwk").filter(|j| j.is_object()) {
        if jwk.get("d").is_some() {
            return Err(format!(
                "verification method {method_id} publishes a private key"
            ));
        }
        return Ok(MethodKey::Jwk(jwk.clone()));
    }
    if let Some(mb) = method.get("publicKeyMultibase").and_then(Value::as_str) {
        return decode_multikey(mb).map(MethodKey::Multikey);
    }
    Err(format!(
        "verification method {method_id} carries no usable public key"
    ))
}

/// An xsd:dateTime / RFC 3339 timestamp (`YYYY-MM-DDThh:mm:ss[.frac](Z|±hh:mm)`; no zone is UTC)
/// as seconds since the epoch.
pub fn parse_datetime(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', 't'])?;
    // Every field must be ASCII digits of a bounded width (so no sign, no overflow on huge years,
    // and nothing for the arithmetic below to wrap on).
    let field = |f: &str, min: usize, max: usize| -> Option<i64> {
        if f.len() < min || f.len() > max || !f.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        f.parse().ok()
    };
    let mut d = date.splitn(3, '-');
    let (y, mo, da) = (
        field(d.next()?, 4, 4)?,
        field(d.next()?, 2, 2)?,
        field(d.next()?, 2, 2)?,
    );
    let zone_at = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (time, zone) = rest.split_at(zone_at);
    let mut time = time.splitn(2, '.');
    let (time, frac) = (time.next()?, time.next());
    if frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let mut t = time.splitn(3, ':');
    let (h, mi, se) = (
        field(t.next()?, 2, 2)?,
        field(t.next()?, 2, 2)?,
        field(t.next()?, 2, 2)?,
    );
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 24 || mi > 59 || se > 60 {
        return None;
    }
    let offset = match zone {
        "" | "Z" | "z" => 0,
        z => {
            let sign = match z.as_bytes().first() {
                Some(b'-') => -1,
                Some(b'+') => 1,
                _ => return None,
            };
            let (oh, om) = z.get(1..)?.split_once(':')?;
            let (oh, om) = (field(oh, 2, 2)?, field(om, 2, 2)?);
            if oh > 23 || om > 59 {
                return None;
            }
            sign * (oh * 3600 + om * 60)
        }
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

// ---------------------------------------------------------------- OpenID Connect

/// The client an ID Token was issued to: its `azp`, which becomes the access token's `client_id`
/// and so MUST be a URI (core section 5.2.3).
pub fn id_token_client(jws: &Jws) -> Result<String, String> {
    let azp = jws
        .claim_str("azp")
        .filter(|s| !s.is_empty())
        .ok_or("the ID Token names no azp, so the client it was issued to is unknown")?;
    if !is_uri(azp) {
        return Err("the ID Token's azp is not a URI".into());
    }
    Ok(azp.to_string())
}

async fn oidc(
    cfg: &LwsConfig,
    http: &reqwest::Client,
    jws: &Jws,
    dpop: &DpopContext<'_>,
) -> Result<Verified, String> {
    let issuer = jws
        .claim_str("iss")
        .filter(|s| !s.is_empty())
        .ok_or("an ID Token needs iss and sub")?
        .to_string();
    let sub = jws
        .claim_str("sub")
        .filter(|s| !s.is_empty())
        .ok_or("an ID Token needs iss and sub")?;
    let suite = Suite::of(jws);
    let subject = match suite {
        // A Solid-OIDC ID Token names its agent in `webid`, which it must carry, whatever shape
        // `sub` has (Solid-OIDC section 8.1.1).
        Suite::SolidOidc => jws
            .claim_str("webid")
            .filter(|w| is_http_url(w))
            .ok_or("a Solid-OIDC ID Token needs a webid that is an http(s) URL")?
            .to_string(),
        Suite::LwsOidc => sub.to_string(),
    };
    let azp = id_token_client(jws)?;
    if !is_http_url(&subject) || !is_http_url(&issuer) {
        return Err("the subject and issuer of an ID Token must be http(s) URLs".into());
    }
    // Every claim requirement, before anything is fetched.
    let admitted = Admitted::claims(cfg, jws, suite, jose::now_secs())?;
    let (content_type, body) = fetch(
        cfg,
        http,
        &subject,
        "application/ld+json, application/cid+json, application/json;q=0.9, text/turtle;q=0.8",
    )
    .await?;
    // The document must name the issuer the way the token's suite asks; naming it the other way
    // too is fine, and naming it only the other way is refused with the reason.
    let links = names_issuer(&content_type, &body, &subject, &issuer);
    match (links.has(suite.issuer_link()), suite.issuer_link()) {
        (true, _) => {}
        (false, IssuerLink::SolidOidcIssuer) if links.open_id_provider => {
            return Err(
                "a Solid-OIDC ID Token's WebID profile must name its solid:oidcIssuer".into(),
            )
        }
        (false, IssuerLink::OpenIdProvider) if links.solid_oidc_issuer => {
            return Err("an ID Token whose subject names a solid:oidcIssuer needs a webid".into())
        }
        (false, _) => {
            return Err(format!(
                "the subject's identity document names no OpenID Provider {issuer}"
            ))
        }
    }
    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let (_, body) = fetch(cfg, http, &discovery_url, "application/json").await?;
    let discovery: Value = serde_json::from_slice(&body)
        .map_err(|_| "the OpenID Provider's discovery document is not JSON")?;
    if !discovery
        .get("issuer")
        .and_then(Value::as_str)
        .is_some_and(|i| i == issuer)
    {
        // Exactly the issuer the token names (OpenID Connect Core 3.1.3.7): every spelling that
        // reaches the same provider is not a provider of its own, to quotas or anything else.
        return Err("the OpenID Provider's discovery document names another issuer".into());
    }
    let jwks_uri = discovery
        .get("jwks_uri")
        .and_then(Value::as_str)
        .ok_or("the OpenID Provider's discovery document names no jwks_uri")?;
    let (_, body) = fetch(
        cfg,
        http,
        jwks_uri,
        "application/jwk-set+json, application/json",
    )
    .await?;
    let jwks: Value =
        serde_json::from_slice(&body).map_err(|_| "the OpenID Provider's JWKS does not parse")?;
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .ok_or("the OpenID Provider's JWKS has no keys")?;
    // Each candidate costs a signature verification, and the provider (any issuer an identity
    // document names) chooses how many keys it publishes: a token without a `kid` is tried
    // against a few of them only, as is a `kid` the set repeats.
    let candidates: Vec<&Value> = match jws.kid() {
        Some(kid) => keys
            .iter()
            .filter(|k| k.get("kid").and_then(Value::as_str) == Some(kid))
            .take(MAX_UNNAMED_KEYS)
            .collect(),
        None => keys.iter().take(MAX_UNNAMED_KEYS).collect(),
    };
    if candidates.is_empty() {
        return Err(format!(
            "the OpenID Provider publishes no key {}",
            jws.kid().unwrap_or_default()
        ));
    }
    if !matches!(jws.alg(), Some("ES256" | "RS256" | "EdDSA")) {
        return Err(format!(
            "unsupported ID Token alg {}",
            jws.alg().unwrap_or_default()
        ));
    }
    if !candidates.iter().any(|k| {
        k.get("use")
            .and_then(Value::as_str)
            .is_none_or(|u| u == "sig")
            && jws.verify_jwk(k)
    }) {
        return Err("the ID Token's signature does not verify".into());
    }
    let owner = ProofOwner {
        trusted: cfg.trusted_oidc_issuers.contains(&issuer),
        issuer: &issuer,
        subject: &subject,
    };
    admitted.possession(cfg, jws, dpop, &owner, jose::now_secs())?;
    Ok(Verified {
        subject,
        client: azp,
    })
}

/// The two OpenID Connect suites an ID Token may come from, told apart by the token alone, before
/// anything is fetched: a Solid-OIDC ID Token carries `webid` or is addressed to `solid`; any
/// other is an LWS OpenID Connect one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suite {
    /// Solid-OIDC: the agent is the `webid`, whose profile names its issuer as
    /// `solid:oidcIssuer`; the token is addressed to `solid` and bound to a key.
    SolidOidc,
    /// LWS OpenID Connect: the agent is the `sub`, a controlled identifier whose document names
    /// its OpenID Provider as a service; the token is addressed to this authorization server.
    LwsOidc,
}

impl Suite {
    #[cfg(test)]
    const ALL: [Suite; 2] = [Suite::SolidOidc, Suite::LwsOidc];

    fn of(jws: &Jws) -> Suite {
        if jws.claims.get("webid").is_some() || jws.audiences().iter().any(|a| a == "solid") {
            Suite::SolidOidc
        } else {
            Suite::LwsOidc
        }
    }

    /// Whether the token is addressed as the suite requires.
    fn addressed(self, cfg: &LwsConfig, jws: &Jws) -> bool {
        match self {
            // "The audience claim MUST be an array of values. The values MUST include the
            // authorized party claim azp and the string solid" (Solid-OIDC, ID Token): every
            // member it requires, in the form it requires.
            Suite::SolidOidc => {
                let audiences = jws.audiences();
                let has = |v: &str| audiences.iter().any(|a| a == v);
                jws.claims.get("aud").is_some_and(Value::is_array)
                    && has("solid")
                    && jws.claim_str("azp").is_some_and(has)
            }
            Suite::LwsOidc => addressed_to_us(cfg, jws),
        }
    }

    /// Whether the suite's tokens must be bound to a key (`cnf.jkt`).
    fn requires_binding(self) -> bool {
        match self {
            Suite::SolidOidc => true,
            Suite::LwsOidc => false,
        }
    }

    /// How the agent's identity document names the token's issuer.
    fn issuer_link(self) -> IssuerLink {
        match self {
            Suite::SolidOidc => IssuerLink::SolidOidcIssuer,
            Suite::LwsOidc => IssuerLink::OpenIdProvider,
        }
    }
}

/// An ID Token that has met every claim requirement of its suite. It is the only way to the proof
/// of possession, so no suite reaches an exchange with a step skipped: a suite chooses what each
/// step demands, never whether the step runs.
struct Admitted {
    suite: Suite,
}

impl Admitted {
    /// Expiry and validity times, the audience, and the key binding, in that order, for every
    /// suite.
    fn claims(cfg: &LwsConfig, jws: &Jws, suite: Suite, now: i64) -> Result<Admitted, String> {
        check_times(jws, now)?;
        if !suite.addressed(cfg, jws) {
            return Err(match suite {
                Suite::SolidOidc => {
                    "a Solid-OIDC ID Token's aud must be an array that includes solid and its azp"
                }
                Suite::LwsOidc => "aud does not include this authorization server",
            }
            .into());
        }
        if suite.requires_binding() && bound_jkt(jws).is_none() {
            return Err("a Solid-OIDC ID Token must be bound to a key (cnf.jkt)".into());
        }
        Ok(Admitted { suite })
    }

    /// The proof of possession, once the signature verifies: a token bound to a key, in any
    /// suite, is exchanged only with a DPoP proof by that key.
    fn possession(
        &self,
        cfg: &LwsConfig,
        jws: &Jws,
        dpop: &DpopContext<'_>,
        owner: &ProofOwner<'_>,
        now: i64,
    ) -> Result<(), String> {
        match bound_jkt(jws) {
            Some(jkt) => check_dpop(
                dpop.proof,
                &cfg.absolute(super::AS_TOKEN_PATH),
                jkt,
                now,
                dpop.replay,
                owner,
            ),
            None if self.suite.requires_binding() => {
                Err("a Solid-OIDC ID Token must be bound to a key (cnf.jkt)".into())
            }
            None => Ok(()),
        }
    }
}

fn is_http_url(s: &str) -> bool {
    (s.starts_with("https://") || s.starts_with("http://")) && url::Url::parse(s).is_ok()
}

fn same_issuer(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// How a subject's identity document names its OpenID Provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuerLink {
    /// A controlled identifier `service` typed `lws:OpenIdProvider`.
    OpenIdProvider,
    /// A Solid WebID's `solid:oidcIssuer`.
    SolidOidcIssuer,
}

/// Which ways an identity document names an issuer: a document may advertise its provider both
/// ways, and each suite asks for its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IssuerLinks {
    pub open_id_provider: bool,
    pub solid_oidc_issuer: bool,
}

impl IssuerLinks {
    pub fn has(self, link: IssuerLink) -> bool {
        match link {
            IssuerLink::OpenIdProvider => self.open_id_provider,
            IssuerLink::SolidOidcIssuer => self.solid_oidc_issuer,
        }
    }
}

/// Every way the identity document of `subject` (a compact CID JSON document, or Turtle) names
/// `issuer` as its OpenID Provider.
pub fn names_issuer(content_type: &str, body: &[u8], subject: &str, issuer: &str) -> IssuerLinks {
    names_issuer_in(content_type, body, subject, issuer).unwrap_or_default()
}

fn names_issuer_in(
    content_type: &str,
    body: &[u8],
    subject: &str,
    issuer: &str,
) -> Option<IssuerLinks> {
    let mut links = IssuerLinks::default();
    let base = subject.split('#').next().unwrap_or(subject);
    if let Ok(doc) = serde_json::from_slice::<Value>(body) {
        // A compact document about the subject whose terms are all defined by a context known
        // here (or that has none: CID documents may be read as plain JSON) is read by its keys.
        // Any other JSON-LD form (expanded, flattened, a graph, an inline or scoped context that
        // could give a key another meaning) is read as RDF, without loading remote contexts, and
        // only when what it can expand to is bounded (see [`expansion_bound`]).
        let about_subject = doc
            .get("id")
            .or_else(|| doc.get("@id"))
            .and_then(Value::as_str)
            == Some(subject);
        if !(about_subject && known_contexts_only(&doc)) {
            if expansion_bound(&doc, body.len(), base) > super::expansion_budget(body.len()) {
                return None;
            }
            let parser = oxjsonld::JsonLdParser::new().with_base_iri(base).ok()?;
            let triples = parser
                .for_slice(body)
                .filter_map(Result::ok)
                .filter(|q| q.graph_name == oxrdf::GraphName::DefaultGraph)
                .map(|q| oxrdf::Triple::new(q.subject, q.predicate, q.object));
            return links_in(triples, body.len(), subject, issuer);
        }
        let services = match doc.get("service") {
            Some(Value::Array(a)) => a.as_slice(),
            Some(o @ Value::Object(_)) => std::slice::from_ref(o),
            _ => &[],
        };
        let provider = services.iter().take(MAX_IDENTITY_SERVICES).any(|s| {
            s.get("type").is_some_and(|t| has_type(t, "OpenIdProvider"))
                && match s.get("serviceEndpoint") {
                    Some(Value::String(e)) => same_issuer(e, issuer),
                    Some(Value::Array(a)) => a
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|e| same_issuer(e, issuer)),
                    _ => false,
                }
        });
        links.open_id_provider = provider;
        // The known contexts do not define `solid:` or `oidcIssuer`: only the full IRI is the
        // Solid term here (a document that defines a shorter one is read as RDF above).
        let solid = [SOLID_OIDC_ISSUER].iter().any(|k| match doc.get(*k) {
            Some(Value::String(e)) => same_issuer(e, issuer),
            Some(Value::Object(o)) => o
                .get("@id")
                .or_else(|| o.get("id"))
                .and_then(Value::as_str)
                .is_some_and(|e| same_issuer(e, issuer)),
            Some(Value::Array(a)) => a.iter().any(|v| {
                v.as_str()
                    .or_else(|| v.get("@id").or_else(|| v.get("id")).and_then(Value::as_str))
                    .is_some_and(|e| same_issuer(e, issuer))
            }),
            _ => false,
        });
        links.solid_oidc_issuer = solid;
        return Some(links);
    }
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !(ct == "text/turtle"
        || ct == "application/n-triples"
        || ct.is_empty()
        || ct == "text/plain")
    {
        return Some(links);
    }
    let parser = oxttl::TurtleParser::new().with_base_iri(base).ok()?;
    links_in(
        parser.for_slice(body).filter_map(Result::ok),
        body.len(),
        subject,
        issuer,
    )
}

/// Remote contexts whose definitions of the keys read from a compact document (`id`, `type`,
/// `service`, `serviceEndpoint`) are the CID ones.
const KNOWN_CONTEXTS: &[&str] = &[
    super::CID_CONTEXT,
    "https://www.w3.org/ns/did/v1",
    super::LWS_CONTEXT,
];

/// Whether every context in `doc` is a [`KNOWN_CONTEXTS`] reference at the top level (or there
/// is none): no inline definition, and no context nested anywhere below, could change what a key
/// means.
fn known_contexts_only(doc: &Value) -> bool {
    let top = match doc.get("@context") {
        None => true,
        Some(Value::String(c)) => KNOWN_CONTEXTS.contains(&c.as_str()),
        Some(Value::Array(a)) => a
            .iter()
            .all(|c| c.as_str().is_some_and(|c| KNOWN_CONTEXTS.contains(&c))),
        Some(_) => false,
    };
    fn nested(v: &Value) -> bool {
        match v {
            Value::Object(o) => o.iter().any(|(k, v)| k == "@context" || nested(v)),
            Value::Array(a) => a.iter().any(nested),
            _ => false,
        }
    }
    let below = match doc {
        Value::Object(o) => o.iter().any(|(k, v)| k != "@context" && nested(v)),
        other => nested(other),
    };
    top && !below
}

/// An upper bound on the bytes `doc` (`len` bytes, read against `base`) can expand to, counted
/// before any expansion. Every expanded term or IRI is at most every inline context's bytes
/// (term definitions chain at most through all of them) plus the base, plus its own bytes in the
/// document; there are at most `len / 2` of them. A remote context is never loaded, so inline
/// ones are all there are.
fn expansion_bound(doc: &Value, len: usize, base: &str) -> usize {
    fn contexts(v: &Value) -> usize {
        match v {
            Value::Object(o) => o
                .iter()
                .map(|(k, v)| {
                    if k == "@context" {
                        serde_json::to_string(v).map_or(usize::MAX, |s| s.len())
                    } else {
                        contexts(v)
                    }
                })
                .fold(0, usize::saturating_add),
            Value::Array(a) => a.iter().map(contexts).fold(0, usize::saturating_add),
            _ => 0,
        }
    }
    (len / 2)
        .saturating_mul(contexts(doc).saturating_add(base.len()))
        .saturating_add(len)
}

/// The issuer links of `subject` among the triples of its identity document (of `len` bytes).
fn links_in(
    parsed: impl Iterator<Item = oxrdf::Triple>,
    len: usize,
    subject: &str,
    issuer: &str,
) -> Option<IssuerLinks> {
    let mut links = IssuerLinks::default();
    // The document is the subject's to write (and is read before any signature is checked), so
    // the work is bounded: so many triples are read, so many services looked at, and the triples
    // are indexed by subject once, so each service costs only its own triples.
    // The expanded terms are bounded too (see [`super::expansion_budget`]): a document whose
    // prefixes (or contexts) expand past it is refused.
    let mut budget = super::expansion_budget(len);
    let mut triples: Vec<oxrdf::Triple> = Vec::new();
    for t in parsed {
        budget = budget.checked_sub(super::triple_bytes(&t))?;
        triples.push(t);
        if triples.len() == MAX_IDENTITY_TRIPLES {
            break;
        }
    }
    let mut by_subject: HashMap<&oxrdf::NamedOrBlankNode, Vec<&oxrdf::Triple>> = HashMap::new();
    for t in &triples {
        by_subject.entry(&t.subject).or_default().push(t);
    }
    fn iri(t: &oxrdf::Term) -> Option<&str> {
        match t {
            oxrdf::Term::NamedNode(n) => Some(n.as_str()),
            _ => None,
        }
    }
    let me = oxrdf::NamedOrBlankNode::NamedNode(oxrdf::NamedNode::new(subject).ok()?);
    let of_subject = by_subject.get(&me).map(Vec::as_slice).unwrap_or_default();
    links.solid_oidc_issuer = of_subject.iter().any(|t| {
        t.predicate.as_str() == SOLID_OIDC_ISSUER
            && iri(&t.object).is_some_and(|o| same_issuer(o, issuer))
    });
    let provider_type = format!("{LWS_NS}OpenIdProvider");
    for t in of_subject
        .iter()
        .filter(|t| t.predicate.as_str() == CID_SERVICE)
        .take(MAX_IDENTITY_SERVICES)
    {
        let service = match &t.object {
            oxrdf::Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n.clone()),
            oxrdf::Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b.clone()),
            _ => continue,
        };
        let about = by_subject
            .get(&service)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let typed = about.iter().any(|u| {
            u.predicate.as_str() == RDF_TYPE && iri(&u.object) == Some(provider_type.as_str())
        });
        let endpoint = about.iter().any(|u| {
            u.predicate.as_str() == CID_SERVICE_ENDPOINT
                && iri(&u.object).is_some_and(|o| same_issuer(o, issuer))
        });
        if typed && endpoint {
            links.open_id_provider = true;
            break;
        }
    }
    Some(links)
}

/// How many triples of an identity document are read.
const MAX_IDENTITY_TRIPLES: usize = 10_000;
/// How many services of an identity document are looked at.
const MAX_IDENTITY_SERVICES: usize = 64;

// ---------------------------------------------------------------- fetching

pub use super::is_forbidden_ip;

/// The scheme and host checks of the fetch policy, without name resolution.
pub fn check_url_static(cfg: &LwsConfig, raw: &str) -> Result<url::Url, String> {
    let u = url::Url::parse(raw).map_err(|_| format!("not a URL: {raw}"))?;
    match u.scheme() {
        "https" => {}
        "http" if cfg.allow_insecure_fetch => {}
        _ => {
            return Err(format!(
                "refusing to fetch {raw}: only https URLs are dereferenced"
            ))
        }
    }
    if cfg.allow_insecure_fetch {
        return Ok(u);
    }
    match u.host() {
        Some(url::Host::Ipv4(ip)) if is_forbidden_ip(IpAddr::V4(ip)) => {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        Some(url::Host::Ipv6(ip)) if is_forbidden_ip(IpAddr::V6(ip)) => {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        Some(url::Host::Domain(d))
            if d.eq_ignore_ascii_case("localhost")
                || d.to_ascii_lowercase().ends_with(".localhost") =>
        {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        None => Err(format!("not a URL with a host: {raw}")),
        _ => Ok(u),
    }
}

/// The full fetch policy: the static checks, then every address the host resolves to.
async fn check_url(cfg: &LwsConfig, raw: &str) -> Result<url::Url, String> {
    let u = check_url_static(cfg, raw)?;
    if cfg.allow_insecure_fetch {
        return Ok(u);
    }
    if let Some(url::Host::Domain(d)) = u.host() {
        let port = u.port_or_known_default().unwrap_or(443);
        let addrs = tokio::net::lookup_host((d, port))
            .await
            .map_err(|_| format!("cannot resolve {d}"))?;
        let mut any = false;
        for a in addrs {
            any = true;
            if is_forbidden_ip(a.ip()) {
                return Err(format!(
                    "refusing to fetch {raw}: {d} resolves to a non-public address"
                ));
            }
        }
        if !any {
            return Err(format!("cannot resolve {d}"));
        }
    }
    Ok(u)
}

/// Redirects a fetch follows, each hop checked against the fetch policy before it is requested.
pub const MAX_REDIRECTS: usize = 3;

/// Where a redirect from `from` with `location` goes, resolved against `from`.
pub fn redirect_target(from: &url::Url, location: Option<&str>) -> Result<url::Url, String> {
    let location = location.ok_or_else(|| format!("{from} redirects without a Location"))?;
    from.join(location)
        .map_err(|_| format!("{from} redirects to an invalid location"))
}

/// GET `url` under the fetch policy: a 200 answer's media type and body (at most 1 MiB).
///
/// `http` must be built by [`super::fetch_client`]: it follows no redirects itself, so each hop is
/// checked (scheme, host and resolved addresses) before it is requested, and its resolver only
/// connects to public addresses, so a name cannot be re-pointed between check and connect.
pub async fn fetch(
    cfg: &LwsConfig,
    http: &reqwest::Client,
    url: &str,
    accept: &str,
) -> Result<(String, Vec<u8>), String> {
    let mut u = check_url(cfg, url).await?;
    u.set_fragment(None);
    let mut hops = 0;
    let mut resp = loop {
        let resp = http
            .get(u.clone())
            .header(reqwest::header::ACCEPT, accept)
            .send()
            .await
            .map_err(|e| format!("cannot dereference {url}: {}", e.without_url()))?;
        if !resp.status().is_redirection() {
            break resp;
        }
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err(format!("{url} redirects too many times"));
        }
        let next = redirect_target(
            &u,
            resp.headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
        )?;
        u = check_url(cfg, next.as_str()).await?;
        u.set_fragment(None);
    };
    if resp.status() != reqwest::StatusCode::OK {
        return Err(format!("GET {url} answered {}", resp.status().as_u16()));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if resp.content_length().is_some_and(|n| n as usize > MAX_DOC) {
        return Err(format!("{url} is too large"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|_| format!("cannot read {url}"))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_DOC {
            return Err(format!("{url} is too large"));
        }
    }
    Ok((content_type, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};

    fn cfg() -> LwsConfig {
        LwsConfig::new("http://localhost:3000")
    }

    fn did_for(key: &jose::EcKey) -> String {
        let point =
            p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(&key.public_key(), true);
        let mut bytes = vec![0x80, 0x24];
        bytes.extend_from_slice(point.as_bytes());
        format!("did:key:z{}", bs58::encode(bytes).into_string())
    }

    fn sign(key: &jose::EcKey, kid: &str, claims: Value) -> Jws {
        // sign_jws sets the key's own kid; build the header by hand to choose it.
        let header = json!({"alg": "ES256", "typ": "JWT", "kid": kid});
        let input = format!(
            "{}.{}",
            jose::b64url(header.to_string().as_bytes()),
            jose::b64url(claims.to_string().as_bytes())
        );
        let sig = key.sign(input.as_bytes());
        Jws::parse(&format!("{input}.{}", jose::b64url(&sig))).unwrap()
    }

    fn claims(did: &str, cfg: &LwsConfig) -> Value {
        let now = jose::now_secs();
        json!({"sub": did, "iss": did, "client_id": did, "aud": [cfg.issuer()], "iat": now, "exp": now + 300})
    }

    fn dpop_proof(key: &jose::EcKey, htm: &str, htu: &str, iat: i64, jti: &str) -> String {
        let mut header = serde_json::Map::new();
        header.insert("typ".into(), json!("dpop+jwt"));
        header.insert("jwk".into(), key.public_jwk());
        key.sign_jws(
            header,
            &json!({"htm": htm, "htu": htu, "iat": iat, "jti": jti}),
        )
    }

    const OWNER: ProofOwner<'static> = ProofOwner {
        issuer: "https://op.example",
        subject: "https://op.example/alice",
        trusted: false,
    };

    /// How many entries of `owner` under `jkt` the cache takes before refusing one, each with a
    /// fresh `jti` of the same length.
    fn fill(replay: &DpopReplay, owner: &ProofOwner<'_>, jkt: &str, now: i64) -> usize {
        let mut held = 0;
        while replay.first_use(owner, jkt, &format!("j{held:06}"), now + 60, now) {
            held += 1;
            assert!(held < 1_000_000, "the share never filled");
        }
        held
    }

    /// The bytes the cache holds, by the reckoning of [`entry_bytes`], checked against what its
    /// share counts say.
    fn held_bytes(replay: &DpopReplay) -> usize {
        let r = replay.0.lock().unwrap();
        let held: usize = r.seen.values().map(|(_, b)| *b).sum();
        let pools = r.used.get(&Share::Pool(false)).copied().unwrap_or(0)
            + r.used.get(&Share::Pool(true)).copied().unwrap_or(0);
        assert_eq!(held, pools);
        held
    }

    /// The one way a document names an issuer, for documents that name it at most one way.
    fn first_link(
        content_type: &str,
        body: &[u8],
        subject: &str,
        issuer: &str,
    ) -> Option<IssuerLink> {
        let links = names_issuer(content_type, body, subject, issuer);
        assert!(
            !(links.open_id_provider && links.solid_oidc_issuer),
            "both ways"
        );
        [IssuerLink::OpenIdProvider, IssuerLink::SolidOidcIssuer]
            .into_iter()
            .find(|l| links.has(*l))
    }

    /// Review finding: a document advertising its provider both ways answered with the first
    /// link found, so a Solid-OIDC token whose WebID also listed a CID service was refused. Every
    /// link the document holds is found, and each suite checks for its own.
    #[test]
    fn documents_may_name_their_issuer_both_ways() {
        let s = "https://alice.example/id";
        let op = "https://op.example";
        let both = IssuerLinks {
            open_id_provider: true,
            solid_oidc_issuer: true,
        };
        let doc = json!({"id": s, SOLID_OIDC_ISSUER: op, "service": [
            {"type": "https://www.w3.org/ns/lws#OpenIdProvider", "serviceEndpoint": op}]});
        assert_eq!(
            names_issuer("application/json", doc.to_string().as_bytes(), s, op),
            both
        );
        let ttl = format!(
            "@prefix cid: <https://www.w3.org/ns/cid/v1#> . <{s}> <{SOLID_OIDC_ISSUER}> <{op}> ; cid:service [ a <https://www.w3.org/ns/lws#OpenIdProvider> ; cid:serviceEndpoint <{op}> ] ."
        );
        assert_eq!(names_issuer("text/turtle", ttl.as_bytes(), s, op), both);
        for suite in Suite::ALL {
            assert!(both.has(suite.issuer_link()), "{suite:?}");
        }
    }

    /// Review finding: one key could fill the whole replay cache, refusing every other client's
    /// proofs until its entries expired, and each refusal scanned the cache. A key holds at most
    /// [`MAX_DPOP_BYTES_PER_KEY`] of live entries, and entries leave in expiry order.
    #[test]
    fn one_key_cannot_fill_the_replay_cache() {
        let replay = DpopReplay::default();
        let now = 1_000;
        let bob = ProofOwner {
            subject: "https://op.example/bob",
            ..OWNER
        };
        let held = fill(&replay, &OWNER, "k", now);
        assert!(
            held >= 16,
            "an ordinary client fits a few dozen proofs, not {held}"
        );
        assert!(held_bytes(&replay) <= MAX_DPOP_BYTES_PER_KEY);
        assert!(replay.first_use(&bob, "other", "j0", now + 60, now));
        // A replay stays refused while it is live, under any identity; once expired its room
        // comes back.
        assert!(!replay.first_use(&OWNER, "k", "j000000", now + 60, now + 30));
        assert!(!replay.first_use(&bob, "k", "j000000", now + 60, now + 30));
        assert!(replay.first_use(&OWNER, "k", "later", now + 200, now + 61));
        assert!(replay.first_use(&OWNER, "k", "j000000", now + 200, now + 61));
        // Everything from before expired; only the two new entries are held, and so counted.
        let r = replay.0.lock().unwrap();
        assert_eq!(r.seen.len(), 2);
        let two: usize = r.seen.values().map(|(_, b)| *b).sum();
        assert_eq!(r.used.get(&Share::Pool(false)), Some(&two));
    }

    /// Review finding: per-key limits do not bound an identity that makes keys, nor a provider
    /// that registers identities. An identity is held to its share whatever keys it uses, a
    /// provider outside the trusted list to its own, and the trusted pool stays open while the
    /// rest is full.
    #[test]
    fn replay_cache_shares_hold_identities_and_providers() {
        let replay = DpopReplay::default();
        let now = 1_000;
        let mut held = 0;
        while replay.first_use(&OWNER, &format!("k{held:06}"), "j", now + 60, now) {
            held += 1;
        }
        assert!(
            held >= 16,
            "an identity fits a few dozen proofs, not {held}"
        );
        assert!(held_bytes(&replay) <= MAX_DPOP_BYTES_PER_IDENTITY);
        // Its provider's other identities fill the provider's share, and no more.
        for i in 0.. {
            let name = format!("https://op.example/u{i:06}");
            let who = ProofOwner {
                subject: &name,
                ..OWNER
            };
            if fill(&replay, &who, &format!("{name}#k"), now) == 0 {
                break;
            }
        }
        assert!(held_bytes(&replay) <= MAX_DPOP_BYTES_PER_OPEN_ISSUER);
        let newcomer = ProofOwner {
            subject: "https://op.example/newcomer",
            ..OWNER
        };
        assert!(!replay.first_use(&newcomer, "nk", "j", now + 60, now));
        let elsewhere = ProofOwner {
            issuer: "https://other.example",
            ..newcomer
        };
        assert!(replay.first_use(&elsewhere, "nk", "j", now + 60, now));
        // Fill the open pool; a trusted provider's identities still get in.
        {
            let mut r = replay.0.lock().unwrap();
            r.used.insert(Share::Pool(false), MAX_DPOP_BYTES_OPEN);
        }
        let third = ProofOwner {
            issuer: "https://third.example",
            ..newcomer
        };
        assert!(!replay.first_use(&third, "tk", "j", now + 60, now));
        let trusted = ProofOwner {
            trusted: true,
            ..third
        };
        assert!(replay.first_use(&trusted, "tk", "j", now + 60, now));
    }

    /// What the cache's tables and strings take up, slots kept for later included.
    fn footprint(replay: &DpopReplay) -> usize {
        let r = replay.0.lock().unwrap();
        let strings: usize = r
            .seen
            .iter()
            .map(|((k, j), (shares, _))| {
                2 * (k.len() + j.len()) + 2 * shares.iter().map(Share::bytes).sum::<usize>()
            })
            .sum();
        r.seen.capacity() * SEEN_SLOT
            + r.used.capacity() * USED_SLOT
            + r.expiry.capacity() * EXPIRY_SLOT
            + strings
    }

    /// Review finding: the byte shares counted live entries only, while the tables kept the slots
    /// of expired ones, so a cache filled once and drained held its peak memory under shares
    /// that read empty. Each entry is charged for the slots it may keep, and a table left much
    /// larger than what it holds is shrunk.
    #[test]
    fn replay_cache_charges_the_slots_it_keeps() {
        let spare = DPOP_SPARE_SLOTS * (SEEN_SLOT + USED_SLOT + EXPIRY_SLOT);
        let replay = DpopReplay::default();
        let mut now = 1_000;
        for round in 0..3 {
            for i in 0..64 {
                let name = format!("https://op.example/u{round}-{i}");
                let who = ProofOwner {
                    subject: &name,
                    ..OWNER
                };
                fill(&replay, &who, &format!("{name}#k"), now);
                assert!(footprint(&replay) <= held_bytes(&replay) + spare);
            }
            // Everything expires; the next proof finds the tables settled.
            now += 120;
            assert!(replay.first_use(&OWNER, "k", &format!("after{round}"), now + 60, now));
            assert!(footprint(&replay) <= held_bytes(&replay) + spare);
            // Held to the allocations themselves, not to what the tables hold.
            let r = replay.0.lock().unwrap();
            let bound = |len: usize| DPOP_SLOTS_PER_ENTRY * len + DPOP_SPARE_SLOTS;
            assert!(
                r.seen.capacity() <= bound(r.seen.len()),
                "{}",
                r.seen.capacity()
            );
            assert!(
                r.used.capacity() <= bound(r.used.len()),
                "{}",
                r.used.capacity()
            );
            assert!(
                r.expiry.capacity() <= bound(r.expiry.len()),
                "{}",
                r.expiry.capacity()
            );
        }
    }

    /// Review finding: a shrink in place may keep a table's allocation, so the bound on what the
    /// replay cache holds rested on the allocator. A settled table is rebuilt at the size of what
    /// it holds; its capacity, whatever it was, comes back within the bound.
    #[test]
    fn settled_tables_are_rebuilt_to_size() {
        let mut table: std::collections::HashMap<u64, u64> = (0..100_000).map(|i| (i, i)).collect();
        table.retain(|k, _| *k < 3);
        let mut heap: std::collections::BinaryHeap<u64> = (0..100_000).collect();
        while heap.len() > 3 {
            heap.pop();
        }
        settle(&mut table);
        settle_heap(&mut heap);
        let bound = DPOP_SLOTS_PER_ENTRY * 3 + DPOP_SPARE_SLOTS;
        assert!(table.capacity() <= bound, "{}", table.capacity());
        assert!(heap.capacity() <= bound, "{}", heap.capacity());
        assert_eq!(table.len(), 3);
        assert_eq!(heap.into_sorted_vec().len(), 3);
    }

    /// Review finding: a DPoP proof's key was matched to the token's `cnf.jkt` by the thumbprint
    /// of its JWK as spelled, so the per-key share went by spelling and one key in many spellings
    /// had many shares. The thumbprint is the key's own; no other spelling of it matches.
    #[test]
    fn dpop_keys_are_known_by_their_own_thumbprint() {
        let htu = "http://localhost:3000/.well-known/lws/token";
        let key = jose::EcKey::generate("c");
        let jkt = key.thumbprint();
        let now = jose::now_secs();
        let replay = DpopReplay::default();
        let proof_with = |jwk: Value, jti: &str| {
            let mut header = serde_json::Map::new();
            header.insert("typ".into(), json!("dpop+jwt"));
            header.insert("jwk".into(), jwk);
            key.sign_jws(
                header,
                &json!({"htm": "POST", "htu": htu, "iat": now, "jti": jti}),
            )
        };
        let jwk = key.public_jwk();
        let padded = {
            let mut j = jwk.clone();
            j["x"] = json!(format!("{}=", jwk["x"].as_str().unwrap()));
            j
        };
        assert_ne!(jose::thumbprint(&padded), jkt);
        assert_eq!(jose::key_thumbprint(&padded).as_deref(), Some(jkt.as_str()));
        assert_eq!(
            check_dpop(Some(&proof_with(jwk, "a")), htu, &jkt, now, &replay, &OWNER),
            Ok(())
        );
        // Another spelling is the same key: the same entries, the same share.
        assert_eq!(
            check_dpop(
                Some(&proof_with(padded.clone(), "b")),
                htu,
                &jkt,
                now,
                &replay,
                &OWNER
            ),
            Ok(())
        );
        assert!(check_dpop(
            Some(&proof_with(padded, "a")),
            htu,
            &jkt,
            now,
            &replay,
            &OWNER
        )
        .is_err());
        let spelled = jose::thumbprint(&{
            let mut j = key.public_jwk();
            j["x"] = json!(format!("{}=", j["x"].as_str().unwrap()));
            j
        });
        let other_spelling = proof_with(key.public_jwk(), "c");
        assert!(check_dpop(Some(&other_spelling), htu, &spelled, now, &replay, &OWNER).is_err());
        // RSA: leading zero bytes do not make another key.
        let rsa = |n: &[u8]| json!({"kty": "RSA", "n": jose::b64url(n), "e": "AQAB"});
        assert_eq!(
            jose::key_thumbprint(&rsa(&[0, 0, 7, 8])),
            jose::key_thumbprint(&rsa(&[7, 8]))
        );
        assert_eq!(
            jose::key_thumbprint(&json!({"kty": "oct", "k": "AA"})),
            None
        );
    }

    /// Review finding: the shares counted entries, not what they hold, so an identity with long
    /// names, or a provider spending them, held far more memory than its count suggested. Shares
    /// are in bytes: long names fill one sooner, and one too long for its share is refused.
    #[test]
    fn replay_cache_shares_count_bytes() {
        let replay = DpopReplay::default();
        let now = 1_000;
        let long = format!("https://op.example/{}", "x".repeat(8 << 10));
        let who = ProofOwner {
            subject: &long,
            ..OWNER
        };
        let held = fill(&replay, &who, "k", now);
        assert!(held < fill(&DpopReplay::default(), &OWNER, "k", now));
        assert!(held_bytes(&replay) <= MAX_DPOP_BYTES_PER_IDENTITY);
        let huge = format!(
            "https://op.example/{}",
            "x".repeat(MAX_DPOP_BYTES_PER_IDENTITY)
        );
        let who = ProofOwner {
            subject: &huge,
            ..OWNER
        };
        assert!(!replay.first_use(&who, "k2", "j", now + 60, now));
        // The whole open pool, whoever fills it, holds no more than its bytes.
        let replay = DpopReplay::default();
        for p in 0.. {
            let issuer = format!("https://op{p:04}.example/{}", "i".repeat(2048));
            let mut any = false;
            for i in 0..64 {
                let name = format!("{issuer}/u{i}");
                let who = ProofOwner {
                    issuer: &issuer,
                    subject: &name,
                    trusted: false,
                };
                if fill(&replay, &who, &format!("{name}#k"), now) > 0 {
                    any = true;
                }
            }
            if !any {
                break;
            }
        }
        assert!(held_bytes(&replay) <= MAX_DPOP_BYTES_OPEN);
        assert!(held_bytes(&replay) > MAX_DPOP_BYTES_OPEN / 2);
    }

    /// Review finding: the triple limit did not bound what the triples hold: a prefix of half a
    /// megabyte used ten thousand times expanded to gigabytes. Expanded terms are held to
    /// [`super::super::expansion_budget`], and a document past it is refused.
    #[test]
    fn identity_documents_are_held_to_an_expansion_budget() {
        let s = "https://alice.example/id";
        let op = "https://op.example";
        let long = format!("https://p.example/{}#", "x".repeat(100_000));
        let uses: String = (0..200)
            .map(|i| format!("<{s}> <urn:p> p:o{i} .\n"))
            .collect();
        let doc = format!("@prefix p: <{long}> .\n<{s}> <http://www.w3.org/ns/solid/terms#oidcIssuer> <{op}> .\n{uses}");
        assert_eq!(first_link("text/turtle", doc.as_bytes(), s, op), None);
        let few: String = (0..5)
            .map(|i| format!("<{s}> <urn:p> p:o{i} .\n"))
            .collect();
        let doc = format!("@prefix p: <{long}> .\n<{s}> <http://www.w3.org/ns/solid/terms#oidcIssuer> <{op}> .\n{few}");
        assert_eq!(
            first_link("text/turtle", doc.as_bytes(), s, op),
            Some(IssuerLink::SolidOidcIssuer)
        );
    }

    #[test]
    fn dpop_proofs_are_checked() {
        let htu = "http://localhost:3000/.well-known/lws/token";
        let key = jose::EcKey::generate("c");
        let jkt = key.thumbprint();
        let now = jose::now_secs();
        let replay = DpopReplay::default();
        let ok = dpop_proof(&key, "POST", htu, now, "a");
        assert_eq!(
            check_dpop(Some(&ok), htu, &jkt, now, &replay, &OWNER),
            Ok(())
        );
        // Replayed, missing, another key, another target or method, stale, a private jwk.
        assert!(check_dpop(Some(&ok), htu, &jkt, now, &replay, &OWNER).is_err());
        assert!(check_dpop(None, htu, &jkt, now, &replay, &OWNER).is_err());
        let other = jose::EcKey::generate("o");
        let p = dpop_proof(&other, "POST", htu, now, "b");
        assert!(check_dpop(Some(&p), htu, &jkt, now, &replay, &OWNER).is_err());
        let p = dpop_proof(&key, "POST", "http://localhost:3000/elsewhere", now, "c");
        assert!(check_dpop(Some(&p), htu, &jkt, now, &replay, &OWNER).is_err());
        let p = dpop_proof(&key, "GET", htu, now, "d");
        assert!(check_dpop(Some(&p), htu, &jkt, now, &replay, &OWNER).is_err());
        let p = dpop_proof(&key, "POST", htu, now - 600, "e");
        assert!(check_dpop(Some(&p), htu, &jkt, now, &replay, &OWNER).is_err());
        let mut header = serde_json::Map::new();
        header.insert("typ".into(), json!("dpop+jwt"));
        header.insert("jwk".into(), key.private_jwk());
        let p = key.sign_jws(
            header,
            &json!({"htm": "POST", "htu": htu, "iat": now, "jti": "f"}),
        );
        assert!(check_dpop(Some(&p), htu, &jkt, now, &replay, &OWNER).is_err());
        // Review finding: an `iat` far enough from now overflowed the freshness check and passed
        // it, with an expiry already past, so the proof could be replayed. Times outside the
        // calendar are no times at all.
        for iat in [i64::MIN + now, i64::MIN, i64::MAX, -1] {
            let p = dpop_proof(&key, "POST", htu, iat, &format!("x{iat}"));
            assert!(
                check_dpop(Some(&p), htu, &jkt, now, &replay, &OWNER).is_err(),
                "{iat}"
            );
        }
        // A query on htu is ignored, as RFC 9449 says.
        let p = dpop_proof(&key, "POST", &format!("{htu}?x=1"), now, "g");
        assert_eq!(
            check_dpop(Some(&p), htu, &jkt, now, &replay, &OWNER),
            Ok(())
        );
    }

    /// Review finding: a Solid-OIDC ID Token (aud `solid`, bound by `cnf.jkt`) was exchanged
    /// without any DPoP proof, so a copied ID Token was as good as the key.
    /// An OpenID Provider at the returned base URL, signing with the returned key, and two agents
    /// it serves: `/alice`, a WebID naming it as `solid:oidcIssuer`, and `/bob`, a controlled
    /// identifier naming it as an `OpenIdProvider` service.
    async fn provider() -> (String, jose::EcKey) {
        let op = jose::EcKey::generate("op-key");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let alice = format!("{base}/alice");
        let bob = json!({"id": format!("{base}/bob"),
            "service": [{"type": "OpenIdProvider", "serviceEndpoint": base}]});
        let discovery = json!({"issuer": base, "jwks_uri": format!("{base}/jwks")});
        let jwks = json!({"keys": [op.public_jwk()]});
        let turtle = format!("<{alice}> <{SOLID_OIDC_ISSUER}> <{base}> .");
        let app = axum::Router::new()
            .route(
                "/alice",
                axum::routing::get(move || async move {
                    ([(axum::http::header::CONTENT_TYPE, "text/turtle")], turtle)
                }),
            )
            .route(
                "/bob",
                axum::routing::get(move || async move { axum::Json(bob) }),
            )
            .route(
                "/.well-known/openid-configuration",
                axum::routing::get(move || async move { axum::Json(discovery) }),
            )
            .route(
                "/jwks",
                axum::routing::get(move || async move { axum::Json(jwks) }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        (base, op)
    }

    /// Review finding: a token carrying `webid` without `solid` among its audiences was taken
    /// for a Solid-OIDC one when its WebID was read, but not when its binding was checked, so it
    /// was exchanged addressed to no one and bound to no key. Every ID Token goes through every
    /// requirement of its suite, whichever way the suite was recognized.
    #[tokio::test]
    async fn every_id_token_meets_every_requirement_of_its_suite() {
        let (base, op) = provider().await;
        let mut cfg = cfg();
        cfg.allow_insecure_fetch = true;
        let http = super::super::fetch_client(&cfg).unwrap();
        let client = jose::EcKey::generate("client");
        let htu = cfg.absolute(super::super::AS_TOKEN_PATH);
        let now = jose::now_secs();
        let replay = DpopReplay::default();
        let mut n = 0;
        let mut exchange = |claims: Value, proof: bool| {
            n += 1;
            let token = op.sign_jws(serde_json::Map::new(), &claims);
            let proof = proof.then(|| dpop_proof(&client, "POST", &htu, now, &format!("p{n}")));
            let (cfg, http, replay) = (&cfg, &http, &replay);
            async move {
                let ctx = DpopContext {
                    proof: proof.as_deref(),
                    replay,
                };
                verify(cfg, http, &token, ID_TOKEN_TYPE, &ctx).await
            }
        };
        let alice = format!("{base}/alice");
        let bob = format!("{base}/bob");
        // Each way a token is recognized as its suite, with the claims that meet every
        // requirement.
        let cases: Vec<(Suite, Value)> = vec![
            (
                Suite::SolidOidc,
                json!({"sub": alice, "webid": alice, "aud": ["solid", "https://app.example/"]}),
            ),
            (
                Suite::SolidOidc,
                json!({"sub": "acct-1", "webid": alice, "aud": ["solid", cfg.issuer(), "https://app.example/"]}),
            ),
            (Suite::LwsOidc, json!({"sub": bob, "aud": [cfg.issuer()]})),
            (Suite::LwsOidc, json!({"sub": bob, "aud": cfg.issuer()})),
        ];
        for suite in Suite::ALL {
            assert!(cases.iter().any(|(s, _)| *s == suite), "{suite:?} untested");
        }
        let full = |extra: &Value, bound: bool| {
            let mut c = json!({"iss": base, "azp": "https://app.example/", "iat": now,
                "exp": now + 300});
            c.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            if bound {
                c["cnf"] = json!({"jkt": client.thumbprint()});
            }
            c
        };
        for (suite, extra) in &cases {
            assert_eq!(
                Suite::of(
                    &Jws::parse(&op.sign_jws(serde_json::Map::new(), &full(extra, true))).unwrap()
                ),
                *suite
            );
            // Bound and proven: exchanged.
            let ok = exchange(full(extra, true), true).await;
            assert!(ok.is_ok(), "{suite:?} {extra}: {ok:?}");
            // Bound, without the proof: refused, in every suite.
            assert!(
                exchange(full(extra, true), false).await.is_err(),
                "{suite:?} {extra}"
            );
            // Unbound: refused for Solid-OIDC, which requires a binding.
            assert_eq!(
                exchange(full(extra, false), false).await.is_ok(),
                *suite == Suite::LwsOidc,
                "{suite:?} {extra}"
            );
            // Expired, without exp, or not yet valid: refused.
            for times in [
                json!({"exp": now - 600}),
                json!({"exp": null}),
                json!({"nbf": now + 600}),
                json!({"iat": now + 600}),
            ] {
                let mut c = full(extra, true);
                for (k, v) in times.as_object().unwrap() {
                    if v.is_null() {
                        c.as_object_mut().unwrap().remove(k);
                    } else {
                        c[k] = v.clone();
                    }
                }
                assert!(
                    exchange(c, true).await.is_err(),
                    "{suite:?} {extra} {times}"
                );
            }
            // Addressed elsewhere, or to no one: refused.
            for aud in [json!(["https://other.example/"]), json!(null)] {
                let mut c = full(extra, true);
                if aud.is_null() {
                    c.as_object_mut().unwrap().remove("aud");
                } else {
                    c["aud"] = aud.clone();
                }
                if *suite == Suite::SolidOidc && c.get("webid").is_none() {
                    continue;
                }
                assert!(exchange(c, true).await.is_err(), "{suite:?} {extra} {aud}");
            }
        }
        // Review finding: a Solid-OIDC token's audience was checked for `solid` alone. It must
        // be an array holding both `solid` and the token's `azp`.
        for aud in [
            json!("solid"),
            json!(["solid"]),
            json!(["solid", "https://other.example/"]),
        ] {
            let c = full(&json!({"sub": alice, "webid": alice, "aud": aud}), true);
            assert!(exchange(c, true).await.is_err(), "{aud}");
        }
        // The finding: a webid without `solid` among the audiences is a Solid-OIDC token all
        // the same, and is refused as one.
        let addressed_to_us = json!({"sub": alice, "webid": alice, "aud": [cfg.issuer()]});
        assert!(exchange(full(&addressed_to_us, false), false)
            .await
            .is_err());
        assert!(exchange(full(&addressed_to_us, true), true).await.is_err());
    }

    #[tokio::test]
    async fn solid_oidc_id_tokens_need_a_dpop_proof() {
        let (base, op) = provider().await;
        let alice = format!("{base}/alice");
        let mut cfg = cfg();
        cfg.allow_insecure_fetch = true;
        let http = super::super::fetch_client(&cfg).unwrap();
        let client = jose::EcKey::generate("client");
        let now = jose::now_secs();
        let id_token = |cnf: bool| {
            let mut claims = json!({"iss": base, "sub": alice, "webid": alice,
                "azp": "https://app.example/", "aud": ["solid", "https://app.example/"], "iat": now, "exp": now + 300});
            if cnf {
                claims["cnf"] = json!({"jkt": client.thumbprint()});
            }
            op.sign_jws(serde_json::Map::new(), &claims)
        };
        let replay = DpopReplay::default();
        let exchange = |token: String, proof: Option<String>| {
            let (cfg, http, replay) = (&cfg, &http, &replay);
            async move {
                let ctx = DpopContext {
                    proof: proof.as_deref(),
                    replay,
                };
                verify(cfg, http, &token, ID_TOKEN_TYPE, &ctx).await
            }
        };
        let htu = cfg.absolute(super::super::AS_TOKEN_PATH);
        // Without a proof, or with one by another key: refused.
        let err = exchange(id_token(true), None).await.unwrap_err();
        assert!(err.contains("DPoP"), "{err}");
        let stolen = dpop_proof(&jose::EcKey::generate("thief"), "POST", &htu, now, "t");
        assert!(exchange(id_token(true), Some(stolen)).await.is_err());
        // A Solid-OIDC token that is not bound to a key: refused.
        let proof = dpop_proof(&client, "POST", &htu, now, "u");
        assert!(exchange(id_token(false), Some(proof)).await.is_err());
        // With the key's proof: exchanged, once.
        let proof = dpop_proof(&client, "POST", &htu, now, "v");
        let ok = exchange(id_token(true), Some(proof.clone())).await.unwrap();
        assert_eq!(ok.subject, alice);
        assert!(exchange(id_token(true), Some(proof)).await.is_err());
        // Review finding: another spelling of the issuer reached the same provider and passed,
        // a provider of its own to every quota. The discovery document's issuer must be the
        // token's exactly.
        let respelled = {
            let claims = json!({"iss": format!("{base}/"), "sub": alice, "webid": alice,
                "azp": "https://app.example/",
                "aud": ["solid", "https://app.example/"], "iat": now, "exp": now + 300, "cnf": {"jkt": client.thumbprint()}});
            op.sign_jws(serde_json::Map::new(), &claims)
        };
        let proof = dpop_proof(&client, "POST", &htu, now, "w");
        let err = exchange(respelled, Some(proof)).await.unwrap_err();
        assert!(err.contains("another issuer"), "{err}");
        // Review finding: a URL-shaped `sub` displaced the token's `webid`. The agent is the
        // `webid` whatever shape `sub` has.
        let account = {
            let claims = json!({"iss": base, "sub": format!("{base}/accounts/123"), "webid": alice,
                "azp": "https://app.example/", "aud": ["solid", "https://app.example/"], "iat": now, "exp": now + 300,
                "cnf": {"jkt": client.thumbprint()}});
            op.sign_jws(serde_json::Map::new(), &claims)
        };
        let proof = dpop_proof(&client, "POST", &htu, now, "x");
        assert_eq!(exchange(account, Some(proof)).await.unwrap().subject, alice);
        // Review finding: a Solid-OIDC token without a `webid` authenticated its `sub`. A
        // Solid-OIDC token must carry a `webid` that is an http(s) URL, and a subject whose
        // profile names a `solid:oidcIssuer` is reached only through one.
        for (aud, webid) in [
            (json!(["solid", "https://app.example/"]), None),
            (json!(["solid", "https://app.example/"]), Some(json!(42))),
            (
                json!(["solid", "https://app.example/"]),
                Some(json!("urn:x:alice")),
            ),
            (json!([cfg.issuer()]), None),
        ] {
            let mut claims = json!({"iss": base, "sub": alice, "azp": "https://app.example/",
                "aud": aud, "iat": now, "exp": now + 300, "cnf": {"jkt": client.thumbprint()}});
            if let Some(w) = webid {
                claims["webid"] = w;
            }
            let token = op.sign_jws(serde_json::Map::new(), &claims);
            let proof = dpop_proof(&client, "POST", &htu, now, &format!("y{claims}"));
            let err = exchange(token, Some(proof)).await.unwrap_err();
            assert!(err.contains("webid"), "{claims}: {err}");
        }
    }

    /// Review finding: a time past the range a claim may hold was read as no claim at all, so a
    /// far-future `nbf` lifted the token's validity restriction. A claim present and out of
    /// range is refused, wherever it is read. Review finding: fractional times, which RFC 7519
    /// allows, were refused; they are read and compared as they are.
    #[test]
    fn out_of_range_times_are_refused_not_ignored() {
        let key = jose::EcKey::generate("x");
        let now = 1_000;
        let at = |extra: Value| {
            let mut claims = json!({"iat": now, "exp": now + 300});
            claims
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            Jws::parse(&key.sign_jws(serde_json::Map::new(), &claims)).unwrap()
        };
        assert_eq!(check_times(&at(json!({})), now), Ok(()));
        assert_eq!(check_times(&at(json!({"nbf": now})), now), Ok(()));
        // Fractions are NumericDates too (RFC 7519 section 2), and are compared as they are.
        for good in [
            json!({"nbf": 1.5}),
            json!({"iat": now as f64 - 0.5}),
            json!({"exp": (now + 300) as f64 + 0.5}),
        ] {
            assert_eq!(check_times(&at(good.clone()), now), Ok(()), "{good}");
        }
        assert!(check_times(&at(json!({"exp": (now - SKEW_SECS) as f64 - 0.5})), now).is_err());
        assert!(check_times(&at(json!({"nbf": (now + SKEW_SECS) as f64 + 0.5})), now).is_err());
        for bad in [
            json!({"exp": (jose::MAX_TIME as f64) + 0.5}),
            json!({"iat": -0.5}),
            json!({"nbf": jose::MAX_TIME + 1}),
            json!({"nbf": -1}),
            json!({"nbf": 1e300}),
            json!({"nbf": "soon"}),
            json!({"exp": jose::MAX_TIME + 1}),
            json!({"iat": i64::MIN}),
        ] {
            assert!(check_times(&at(bad.clone()), now).is_err(), "{bad}");
        }
    }

    /// Review finding: a compact document was read by its keys whatever its context said, so a
    /// context could give `oidcIssuer` (or `service`) another meaning and still be read as the
    /// Solid (or CID) term; and a document was expanded with no bound on what a short inline
    /// context could make of it. Only known contexts are read by keys, and a document that could
    /// expand past the budget is refused before it is expanded.
    #[test]
    fn identity_documents_are_read_as_their_contexts_say() {
        let me = "https://alice.example/#me";
        let op = "https://op.example/";
        // An inline context that maps `oidcIssuer` to an unrelated term.
        let remapped = json!({
            "@context": {"oidcIssuer": "https://example.org/notTheIssuer"},
            "id": me,
            "oidcIssuer": op,
        })
        .to_string();
        assert!(
            !names_issuer("application/ld+json", remapped.as_bytes(), me, op).solid_oidc_issuer
        );
        // A scoped context below the top level is not read by keys either.
        let scoped = json!({
            "@context": super::super::CID_CONTEXT,
            "id": me,
            "service": [{
                "@context": {"serviceEndpoint": "https://example.org/elsewhere"},
                "type": "OpenIdProvider",
                "serviceEndpoint": op,
            }],
        })
        .to_string();
        assert!(!names_issuer("application/ld+json", scoped.as_bytes(), me, op).open_id_provider);
        // The full Solid IRI is read by key under a known context.
        let known = json!({
            "@context": super::super::CID_CONTEXT,
            "id": me,
            SOLID_OIDC_ISSUER: {"id": op},
        })
        .to_string();
        assert!(names_issuer("application/ld+json", known.as_bytes(), me, op).solid_oidc_issuer);
        // A short document whose inline context defines a long term, used many times, could
        // expand far past its size: it is refused before it is expanded.
        let long = format!("https://e.example/{}", "x".repeat(8 * 1024));
        let mut amplified = serde_json::Map::new();
        amplified.insert("@context".into(), json!({"t": long}));
        amplified.insert("@id".into(), json!(me));
        amplified.insert(
            "t".into(),
            Value::Array((0..4096).map(|i| json!({"t": i})).collect()),
        );
        let amplified = Value::Object(amplified);
        let body = amplified.to_string();
        assert!(
            expansion_bound(&amplified, body.len(), me)
                > super::super::expansion_budget(body.len())
        );
        assert_eq!(
            names_issuer("application/ld+json", body.as_bytes(), me, op),
            IssuerLinks::default()
        );
    }

    /// Review finding: an identity document served as expanded JSON-LD (the representation the
    /// fetch prefers) named no issuer, because only compact documents were read.
    #[test]
    fn expanded_json_ld_profiles_name_their_issuer() {
        let me = "https://alice.example/#me";
        let op = "https://op.example/";
        let expanded = json!([{
            "@id": me,
            SOLID_OIDC_ISSUER: [{"@id": op}],
        }])
        .to_string();
        let links = names_issuer("application/ld+json", expanded.as_bytes(), me, op);
        assert!(links.solid_oidc_issuer);
        // A graph with an inline context reads the same.
        let graph = json!({
            "@context": {"solid": "http://www.w3.org/ns/solid/terms#"},
            "@graph": [{"@id": me, "solid:oidcIssuer": {"@id": op}}],
        })
        .to_string();
        assert!(names_issuer("application/ld+json", graph.as_bytes(), me, op).solid_oidc_issuer);
        // Naming another subject, or another issuer, is no link.
        assert!(
            !names_issuer(
                "application/ld+json",
                expanded.as_bytes(),
                "https://bob.example/#me",
                op
            )
            .solid_oidc_issuer
        );
        assert!(
            !names_issuer(
                "application/ld+json",
                expanded.as_bytes(),
                me,
                "https://other.example/"
            )
            .solid_oidc_issuer
        );
    }

    #[test]
    fn id_token_azp_must_be_a_uri() {
        let key = jose::EcKey::generate("x");
        let with = |azp: Value| sign(&key, "x", json!({"azp": azp}));
        assert_eq!(
            id_token_client(&with(json!("https://app.example/id"))).unwrap(),
            "https://app.example/id"
        );
        assert!(id_token_client(&with(json!("my-app"))).is_err());
        assert!(id_token_client(&with(json!(""))).is_err());
        assert!(id_token_client(&sign(&key, "x", json!({}))).is_err());
    }

    #[test]
    fn redirect_hops_resolve_against_the_current_url() {
        let from = url::Url::parse("https://id.example/a/b").unwrap();
        assert_eq!(
            redirect_target(&from, Some("/c")).unwrap().as_str(),
            "https://id.example/c"
        );
        assert_eq!(
            redirect_target(&from, Some("http://127.0.0.1/x"))
                .unwrap()
                .as_str(),
            "http://127.0.0.1/x"
        );
        assert!(redirect_target(&from, None).is_err());
        // Each hop then faces the policy: a redirect into loopback is refused before it is
        // requested.
        let cfg = cfg();
        assert!(check_url_static(&cfg, "http://127.0.0.1/x").is_err());
        assert!(check_url_static(&cfg, "https://127.0.0.1/x").is_err());
        assert!(check_url_static(&cfg, "https://[::1]/x").is_err());
    }

    /// A redirect to a loopback address is never requested: the fetch client follows no
    /// redirect, and the hop fails the policy check before any request goes out.
    #[tokio::test]
    async fn fetch_refuses_a_redirect_into_loopback() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // The internal target: counts the requests that reach it.
        let internal = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let internal_addr = internal.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = internal.accept().await {
                h.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                    .await;
            }
        });
        // The "attacker's" server redirects there.
        let outer = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let outer_addr = outer.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = outer.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nlocation: http://{internal_addr}/secret\r\ncontent-length: 0\r\n\r\n"
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        // The first hop is let through (as the insecure escape hatch would), but the policy is the
        // secure one for every hop after: model that by checking the redirect against a secure
        // configuration with the client the insecure one builds.
        let mut insecure = cfg();
        insecure.allow_insecure_fetch = true;
        let client = super::super::fetch_client(&insecure).unwrap();
        let resp = client
            .get(format!("http://{outer_addr}/me"))
            .send()
            .await
            .unwrap();
        // The client does not follow the redirect itself.
        assert_eq!(resp.status().as_u16(), 302);
        let next = redirect_target(
            &url::Url::parse(&format!("http://{outer_addr}/me")).unwrap(),
            resp.headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
        )
        .unwrap();
        assert!(check_url(&cfg(), next.as_str()).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // And a secure fetch of a non-https or loopback URL never leaves the process.
        assert!(
            fetch(&cfg(), &client, &format!("http://{outer_addr}/me"), "*/*")
                .await
                .is_err()
        );
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_resolver_keeps_public_addresses_only() {
        let addrs: Vec<std::net::SocketAddr> = [
            "127.0.0.1:443",
            "10.0.0.1:443",
            "169.254.169.254:80",
            "[::1]:443",
            "93.184.216.34:443",
        ]
        .iter()
        .map(|a| a.parse().unwrap())
        .collect();
        assert_eq!(
            super::super::public_addrs(addrs),
            vec!["93.184.216.34:443".parse::<std::net::SocketAddr>().unwrap()]
        );
    }

    #[test]
    fn did_key_p256_decodes_and_verifies() {
        let cfg = cfg();
        let key = jose::EcKey::generate("x");
        let did = did_for(&key);
        let mb = did.strip_prefix("did:key:").unwrap().to_string();
        match did_key_public(&did).unwrap() {
            MultikeyPublic::P256(k) => assert_eq!(k, key.public_key()),
            other => panic!("{other:?}"),
        }
        let jws = sign(&key, &mb, claims(&did, &cfg));
        assert_eq!(
            did_key(&cfg, &jws).unwrap(),
            Verified {
                subject: did.clone(),
                client: did.clone()
            }
        );
        let jws = sign(&key, &format!("{did}#{mb}"), claims(&did, &cfg));
        assert!(did_key(&cfg, &jws).is_ok());
        // Wrong kid, wrong key.
        assert!(did_key(&cfg, &sign(&key, "other", claims(&did, &cfg))).is_err());
        let other = jose::EcKey::generate("y");
        assert!(did_key(&cfg, &sign(&other, &mb, claims(&did, &cfg))).is_err());
    }

    #[test]
    fn ed25519_did_key_decodes() {
        // did:key test vector from the did:key specification.
        let did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        assert!(matches!(
            did_key_public(did).unwrap(),
            MultikeyPublic::Ed25519(_)
        ));
        assert!(did_key_public("did:key:zQ3s").is_err());
        assert!(did_key_public("did:web:example.org").is_err());
        // Review finding: an oversized multikey was base58-decoded in full (quadratic) before any
        // length check. It is refused up front, and a value under the cap that decodes past the
        // largest key overflows the fixed buffer instead of growing one.
        let huge = format!("did:key:z{}", "2".repeat(100_000));
        let started = std::time::Instant::now();
        let err = did_key_public(&huge).unwrap_err();
        assert!(err.contains("longer"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        let long = format!("z{}", "2".repeat(MAX_MULTIKEY_CHARS));
        assert!(decode_multikey(&long).unwrap_err().contains("longer"));
        assert!(did_key_kid_matches(
            did,
            Some("z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK")
        ));
        assert!(!did_key_kid_matches(did, None));
    }

    #[test]
    fn self_issued_claim_faults() {
        let cfg = cfg();
        let key = jose::EcKey::generate("x");
        let did = did_for(&key);
        let now = jose::now_secs();
        let check = |edit: &dyn Fn(&mut Map<String, Value>)| {
            let mut c = claims(&did, &cfg);
            edit(c.as_object_mut().unwrap());
            self_issued(&cfg, &sign(&key, "k", c), now)
        };
        assert!(check(&|_| {}).is_ok());
        assert!(check(&|c| {
            c.remove("sub");
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("iss");
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("client_id");
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("client_id".into(), json!("https://other.example/"));
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("exp");
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("iat");
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("exp".into(), json!(now - 3600));
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("iat".into(), json!(now + 3600));
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("aud".into(), json!(["https://elsewhere.example"]));
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("aud");
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("aud".into(), json!(format!("{}/", cfg.issuer())));
        })
        .is_ok());
    }

    #[test]
    fn alg_none_is_refused() {
        let t = format!(
            "{}.{}.",
            jose::b64url(br#"{"alg":"none"}"#),
            jose::b64url(br#"{"sub":"a"}"#)
        );
        assert!(parse(&t).is_err());
        assert!(parse("not.a").is_err());
    }

    fn cid_doc(subject: &str, jwk: Value) -> Value {
        json!({
            "id": subject,
            "authentication": [{"id": format!("{subject}#k"), "type": "JsonWebKey", "controller": subject, "publicKeyJwk": jwk}],
        })
    }

    /// Sweep finding: each `authentication` reference was looked up by scanning every
    /// `verificationMethod`, so a document of a hundred thousand of each, fetched before any
    /// signature is checked, cost billions of comparisons. Each list is now read once.
    #[test]
    fn cid_lookup_reads_each_list_once() {
        let s = "https://alice.example/id";
        let key = jose::EcKey::generate("k");
        let jwk = jose::public_jwk_of(&key.public_key());
        let n = 50_000;
        let mut refs: Vec<Value> = (0..n).map(|i| json!(format!("#x{i}"))).collect();
        let mut methods: Vec<Value> = (0..n)
            .map(|i| json!({"id": format!("#y{i}"), "controller": s, "publicKeyJwk": jwk}))
            .collect();
        refs.push(json!("#k"));
        methods.push(json!({"id": "#k", "controller": s, "publicKeyJwk": jwk}));
        let doc = json!({"id": s, "authentication": refs, "verificationMethod": methods});
        let started = std::time::Instant::now();
        assert!(cid_method_key(&doc, s, "k", jose::now_secs()).is_ok());
        assert!(cid_method_key(&doc, s, "nope", jose::now_secs()).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // A reference to a method the document does not define gives way to an embedded one.
        let doc = json!({"id": s, "authentication": ["#k", {"id": "#k", "controller": s, "publicKeyJwk": jwk}]});
        assert!(cid_method_key(&doc, s, "k", jose::now_secs()).is_ok());
    }

    #[test]
    fn cid_method_rules() {
        let s = "https://alice.example/id";
        let key = jose::EcKey::generate("k");
        let jwk = jose::public_jwk_of(&key.public_key());
        let now = jose::now_secs();
        assert!(cid_method_key(&cid_doc(s, jwk.clone()), s, "k", now).is_ok());
        assert!(cid_method_key(&cid_doc(s, jwk.clone()), s, "#k", now).is_ok());
        assert!(cid_method_key(&cid_doc(s, jwk.clone()), s, &format!("{s}#k"), now).is_ok());
        assert!(cid_method_key(&cid_doc(s, jwk.clone()), s, "nope", now).is_err());
        // Another id.
        assert!(
            cid_method_key(&cid_doc("https://bob.example/id", jwk.clone()), s, "k", now).is_err()
        );
        // Referenced method.
        let doc = json!({"id": s, "authentication": ["#k"], "verificationMethod": [{"id": "#k", "controller": s, "publicKeyJwk": jwk}]});
        assert!(cid_method_key(&doc, s, "k", now).is_ok());
        // Listed only for assertion.
        let doc = json!({"id": s, "assertionMethod": ["#k"], "verificationMethod": [{"id": "#k", "controller": s, "publicKeyJwk": jwk}]});
        assert!(cid_method_key(&doc, s, "k", now).is_err());
        // Foreign controller, revoked, expired in the future is fine.
        let mut doc = cid_doc(s, jwk.clone());
        doc["authentication"][0]["controller"] = json!("https://mallory.example/id");
        assert!(cid_method_key(&doc, s, "k", now).is_err());
        let mut doc = cid_doc(s, jwk.clone());
        doc["authentication"][0]["revoked"] = json!("2020-01-01T00:00:00Z");
        assert!(cid_method_key(&doc, s, "k", now).is_err());
        let mut doc = cid_doc(s, jwk);
        doc["authentication"][0]["expires"] = json!("2999-01-01T00:00:00Z");
        assert!(cid_method_key(&doc, s, "k", now).is_ok());
    }

    #[test]
    fn datetimes() {
        assert_eq!(parse_datetime("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_datetime("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(
            parse_datetime("2000-03-01T01:00:00.123+01:00"),
            Some(951_868_800)
        );
        assert_eq!(
            parse_datetime("2000-02-29T23:00:00-01:00"),
            Some(951_868_800)
        );
        assert_eq!(parse_datetime("yesterday"), None);
        // Malformed (non-ASCII, signed, oversized) fields are `None`, never a panic or overflow.
        for bad in [
            "2000-03-01T00:00:00+0\u{e9}:00",
            "2000-03-01T00:00:00Zjunk",
            "2000-03-01T00:00:00.Z",
            "2000-+3-01T00:00:00Z",
            "99999999999999999999-03-01T00:00:00Z",
            "9223372036854775807-03-01T00:00:00Z",
            "2000-03-01T00:00:00+05",
        ] {
            assert_eq!(parse_datetime(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn issuer_links() {
        let s = "https://alice.example/id";
        let op = "https://op.example";
        let doc = json!({"id": s, "service": [{"type": "https://www.w3.org/ns/lws#OpenIdProvider", "serviceEndpoint": op}]});
        let body = doc.to_string();
        assert_eq!(
            first_link("application/json", body.as_bytes(), s, op),
            Some(IssuerLink::OpenIdProvider)
        );
        assert_eq!(
            first_link(
                "application/json",
                body.as_bytes(),
                s,
                "https://evil.example"
            ),
            None
        );
        let ttl = format!("<{s}> <{SOLID_OIDC_ISSUER}> <{op}/> .");
        assert_eq!(
            first_link("text/turtle", ttl.as_bytes(), s, op),
            Some(IssuerLink::SolidOidcIssuer)
        );
        let ttl = format!(
            "@prefix cid: <https://www.w3.org/ns/cid/v1#> . <{s}> cid:service [ a <https://www.w3.org/ns/lws#OpenIdProvider> ; cid:serviceEndpoint <{op}> ] ."
        );
        assert_eq!(
            first_link("text/turtle", ttl.as_bytes(), s, op),
            Some(IssuerLink::OpenIdProvider)
        );
        assert_eq!(
            first_link("text/turtle", ttl.as_bytes(), "https://bob.example/id", op),
            None
        );
    }

    /// Review finding: each `cid:service` link scanned every triple of the document, with an
    /// allocation per comparison, so a document of many services was quadratic work, done before
    /// any signature is checked. The triples are indexed once and the work is capped.
    #[test]
    fn issuer_lookup_is_bounded() {
        let s = "https://alice.example/id";
        let op = "https://op.example";
        let provider = format!(
            "<{s}> cid:service [ a <https://www.w3.org/ns/lws#OpenIdProvider> ; cid:serviceEndpoint <{op}> ] ."
        );
        let filler = |n: usize| {
            (0..n)
                .map(|i| format!("<{s}> cid:service _:s{i} . _:s{i} a <https://e.example/T{i}> ; <https://e.example/p> \"{i}\" .\n"))
                .collect::<String>()
        };
        let doc = |services: usize, provider_first: bool| {
            let head = "@prefix cid: <https://www.w3.org/ns/cid/v1#> .\n";
            if provider_first {
                format!("{head}{provider}\n{}", filler(services))
            } else {
                format!("{head}{}{provider}\n", filler(services))
            }
        };
        // Within the caps the provider is found wherever it is.
        let ttl = doc(MAX_IDENTITY_SERVICES - 1, false);
        assert_eq!(
            first_link("text/turtle", ttl.as_bytes(), s, op),
            Some(IssuerLink::OpenIdProvider)
        );
        // Thousands of services: quick, and only the first ones are looked at.
        let ttl = doc(3000, true);
        let started = std::time::Instant::now();
        assert_eq!(
            first_link("text/turtle", ttl.as_bytes(), s, op),
            Some(IssuerLink::OpenIdProvider)
        );
        let ttl = doc(3000, false);
        assert_eq!(first_link("text/turtle", ttl.as_bytes(), s, op), None);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        // JSON: the same cap on services.
        let mut services: Vec<Value> = (0..MAX_IDENTITY_SERVICES)
            .map(|i| json!({"type": "Other", "serviceEndpoint": format!("https://x{i}.example")}))
            .collect();
        services.push(
            json!({"type": "https://www.w3.org/ns/lws#OpenIdProvider", "serviceEndpoint": op}),
        );
        let body = json!({"id": s, "service": services}).to_string();
        assert_eq!(first_link("application/json", body.as_bytes(), s, op), None);
    }

    #[test]
    fn fetch_policy() {
        let mut cfg = cfg();
        assert!(check_url_static(&cfg, "http://example.org/").is_err());
        assert!(check_url_static(&cfg, "https://127.0.0.1/").is_err());
        assert!(check_url_static(&cfg, "https://[::1]/").is_err());
        assert!(check_url_static(&cfg, "https://10.1.2.3/").is_err());
        assert!(check_url_static(&cfg, "https://169.254.169.254/").is_err());
        assert!(check_url_static(&cfg, "https://localhost/").is_err());
        assert!(check_url_static(&cfg, "file:///etc/passwd").is_err());
        assert!(check_url_static(&cfg, "https://example.org/").is_ok());
        assert!(check_url_static(&cfg, "https://8.8.8.8/").is_ok());
        cfg.allow_insecure_fetch = true;
        assert!(check_url_static(&cfg, "http://localhost:3918/agents/alice").is_ok());
        assert!(check_url_static(&cfg, "file:///etc/passwd").is_err());
    }
}
