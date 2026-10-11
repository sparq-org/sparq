//! The OpenID Connect authentication suites of the Linked Web Storage authorization server
//! (`lws10-authn-openid`, and Solid-OIDC as the Community Solid Server accepts it): an ID Token as
//! the subject token.
//!
//! The subject's identity document must name the token's issuer, as an `lws:OpenIdProvider`
//! service or (Solid-OIDC) with `solid:oidcIssuer`; the issuer's key comes from OpenID Connect
//! Discovery; `azp` names the client. A Solid-OIDC ID Token (one addressed to `solid`, or bound
//! to a key by `cnf.jkt`) is exchanged only with a DPoP proof of that key on the token request
//! (RFC 9449 section 4.3, see [`check_dpop`]): a copied ID Token is worth nothing without it.
//!
//! Documents are fetched under the server's fetch policy (see [`sparq_lws_authn::fetch`]).
//! [`OpenId`] is the suite, for a server's [`Suites`](sparq_lws_authn::Suites).
//! A server built without this suite may let another suite take the ID Token type.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use serde_json::Value;

use sparq_lws_authn::fetch::Policy;
use sparq_lws_authn::jose::{self, Jws};
pub use sparq_lws_authn::ID_TOKEN_TYPE;
use sparq_lws_authn::{
    check_times, is_uri, Exchange, Suite as AuthnSuite, Verified, CID_CONTEXT, LWS_CONTEXT, LWS_NS,
};

/// The environment variable listing the trusted OpenID Providers (see [`OpenId::from_env`]).
pub const TRUSTED_ISSUERS_VAR: &str = "SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS";

const SOLID_OIDC_ISSUER: &str = "http://www.w3.org/ns/solid/terms#oidcIssuer";
const CID_SERVICE: &str = "https://www.w3.org/ns/cid/v1#service";
const CID_SERVICE_ENDPOINT: &str = "https://www.w3.org/ns/cid/v1#serviceEndpoint";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The OpenID Connect suites: the providers whose identities get the reserved half of the DPoP
/// replay cache, and the cache.
#[derive(Default)]
pub struct OpenId {
    trusted_issuers: Vec<String>,
    replay: DpopReplay,
}

impl OpenId {
    /// The suite, with `trusted_issuers` (exactly as their ID Tokens' `iss`) getting the
    /// reserved half of the DPoP replay cache; every other provider shares the other half.
    pub fn new(trusted_issuers: Vec<String>) -> Self {
        Self {
            trusted_issuers,
            replay: DpopReplay::default(),
        }
    }

    /// The suite the environment configures: [`TRUSTED_ISSUERS_VAR`], comma-separated.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var(TRUSTED_ISSUERS_VAR)
                .ok()
                .map(|v| {
                    v.split(',')
                        .map(|i| i.trim().to_string())
                        .filter(|i| !i.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        )
    }
}

impl std::fmt::Debug for OpenId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenId")
            .field("trusted_issuers", &self.trusted_issuers)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl AuthnSuite for OpenId {
    fn name(&self) -> &'static str {
        "openid"
    }

    fn token_types(&self) -> &[&'static str] {
        &[ID_TOKEN_TYPE]
    }

    fn identifier_types(&self) -> &[&'static str] {
        &["https"]
    }

    async fn verify(&self, exchange: &Exchange<'_>) -> Result<Verified, String> {
        let cfg = Ctx {
            issuer: exchange.authorization_server,
            token_endpoint: exchange.token_endpoint,
            allow_insecure_fetch: exchange.fetch.allow_insecure_fetch,
            trusted_oidc_issuers: &self.trusted_issuers,
        };
        let jws = sparq_lws_authn::parse_jwt(exchange.subject_token)?;
        oidc(
            &cfg,
            exchange.http,
            &jws,
            &DpopContext {
                proof: exchange.dpop,
                replay: &self.replay,
            },
        )
        .await
    }
}

/// One exchange's view of the authorization server.
struct Ctx<'a> {
    issuer: &'a str,
    token_endpoint: &'a str,
    allow_insecure_fetch: bool,
    trusted_oidc_issuers: &'a [String],
}

impl Ctx<'_> {
    fn issuer(&self) -> &str {
        self.issuer
    }

    fn policy(&self) -> Policy {
        Policy {
            allow_insecure_fetch: self.allow_insecure_fetch,
        }
    }
}

/// Whether `aud` names this authorization server.
fn addressed_to_us(cfg: &Ctx<'_>, jws: &Jws) -> bool {
    sparq_lws_authn::addressed_to(cfg.issuer(), jws)
}

/// GET `url` under the fetch policy (see [`sparq_lws_authn::fetch::fetch`]).
async fn fetch(
    cfg: &Ctx<'_>,
    http: &reqwest::Client,
    url: &str,
    accept: &str,
) -> Result<(String, Vec<u8>), String> {
    sparq_lws_authn::fetch::fetch(&cfg.policy(), http, url, accept).await
}

/// Whether a JSON `type` value (a string or an array) names `wanted`, short or in the LWS
/// namespace.
fn has_type(v: &Value, wanted: &str) -> bool {
    let matches = |t: &Value| {
        t.as_str()
            .is_some_and(|t| t == wanted || t.strip_prefix(LWS_NS) == Some(wanted))
    };
    match v {
        Value::Array(a) => a.iter().any(matches),
        other => matches(other),
    }
}

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
/// belongs to, by the bytes it holds (see `MAX_DPOP_BYTES_TRUSTED`). Expired entries leave in expiry order, a few at a
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
    cfg: &Ctx<'_>,
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
        "text/turtle, application/cid+json;q=0.9, application/ld+json;q=0.8, application/json;q=0.7",
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
        expires: sparq_lws_authn::jwt_expires(jws)?,
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
    fn addressed(self, cfg: &Ctx<'_>, jws: &Jws) -> bool {
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
    fn claims(cfg: &Ctx<'_>, jws: &Jws, suite: Suite, now: i64) -> Result<Admitted, String> {
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
        cfg: &Ctx<'_>,
        jws: &Jws,
        dpop: &DpopContext<'_>,
        owner: &ProofOwner<'_>,
        now: i64,
    ) -> Result<(), String> {
        match bound_jkt(jws) {
            Some(jkt) => check_dpop(dpop.proof, cfg.token_endpoint, jkt, now, dpop.replay, owner),
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
        // Only a compact document about the subject whose terms are all defined by a context
        // known here (or that has none: CID documents may be read as plain JSON) is read, by its
        // keys. Any other JSON-LD form (expanded, flattened, a graph, an inline or scoped
        // context that could give a key another meaning) names no issuer: the document is the
        // subject's to write and is read before any signature is checked, and general JSON-LD
        // expansion cannot be bounded before it allocates, so it is never run on it.
        let about_subject = doc
            .get("id")
            .or_else(|| doc.get("@id"))
            .and_then(Value::as_str)
            == Some(subject);
        if !(about_subject && known_contexts_only(&doc)) {
            return None;
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
        // Its value must be a node reference (`{"id": …}`): no known context coerces the
        // property to `@id`, so a bare string is a literal, as the RDF reading below finds it.
        let node = |v: &Value| {
            v.get("@id")
                .or_else(|| v.get("id"))
                .and_then(Value::as_str)
                .is_some_and(|e| same_issuer(e, issuer))
        };
        let solid = match doc.get(SOLID_OIDC_ISSUER) {
            Some(v @ Value::Object(_)) => node(v),
            Some(Value::Array(a)) => a.iter().any(node),
            _ => false,
        };
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
    // Before it yields a triple, the parser holds a term for every level of nesting open and an
    // expanded IRI for every prefix declared, so a short document could make it take far more
    // than the expansion budget before that budget sees anything. What it can hold is bounded
    // before it runs (see [`turtle_state_bounded`]), and parsing stops at the first error, so the
    // parser never resynchronises past what was counted.
    if !turtle_state_bounded(body, base.len()) {
        return None;
    }
    let parser = oxttl::TurtleParser::new().with_base_iri(base).ok()?;
    links_in(
        parser.for_slice(body).map_while(Result::ok),
        body.len(),
        subject,
        issuer,
    )
}

/// Most bytes the Turtle parser may hold, by [`turtle_state_bounded`]'s count, before it yields
/// a triple.
const MAX_TURTLE_STATE: usize = 32 << 20;

/// Whether what the Turtle parser can hold of `body` (read against a base of `base_len` bytes)
/// stays within [`MAX_TURTLE_STATE`], counted without reading the grammar, so no escape or
/// unusual token can hide from it. Every term the parser holds is at most one IRI resolved
/// against the base in force, and no base (each relative to the last) or expanded name is longer
/// than the base, plus the document twice over. It holds one such term for each place that
/// opens a level of nesting (a `[`, `(`, `<<` or `{` byte anywhere, which over-counts those in
/// IRIs, strings and comments) and one for each directive (any `prefix` or `base`, in any
/// case, anywhere).
fn turtle_state_bounded(body: &[u8], base_len: usize) -> bool {
    let mut held = 0usize;
    for (i, &c) in body.iter().enumerate() {
        let rest = &body[i..];
        let opens = matches!(c, b'[' | b'(' | b'{') || rest.starts_with(b"<<");
        let directive =
            |word: &[u8]| rest.len() >= word.len() && rest[..word.len()].eq_ignore_ascii_case(word);
        if opens || directive(b"prefix") || directive(b"base") {
            held += 1;
        }
    }
    let term = base_len.saturating_add(body.len().saturating_mul(2)).max(1);
    held.saturating_add(1).saturating_mul(term) <= MAX_TURTLE_STATE
}

/// Remote contexts whose definitions of the keys read from a compact document (`id`, `type`,
/// `service`, `serviceEndpoint`) are the CID ones.
const KNOWN_CONTEXTS: &[&str] = &[CID_CONTEXT, "https://www.w3.org/ns/did/v1", LWS_CONTEXT];

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
    // The expanded terms are bounded too (see [`sparq_lws_authn::expansion_budget`]): a document whose
    // prefixes (or contexts) expand past it is refused.
    let mut budget = sparq_lws_authn::expansion_budget(len);
    let mut triples: Vec<oxrdf::Triple> = Vec::new();
    for t in parsed {
        budget = budget.checked_sub(sparq_lws_authn::triple_bytes(&t))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sparq_lws_authn::fetch::{fetch_client, MAX_DOC};

    /// The authorization server the tests exchange at.
    fn cfg() -> Ctx<'static> {
        Ctx {
            issuer: "http://localhost:3000",
            token_endpoint: "http://localhost:3000/.well-known/lws/token",
            allow_insecure_fetch: false,
            trusted_oidc_issuers: &[],
        }
    }

    /// The exchange of an ID Token at `cfg`, as the token endpoint makes it.
    async fn verify(
        cfg: &Ctx<'_>,
        http: &reqwest::Client,
        token: &str,
        token_type: &str,
        dpop: &DpopContext<'_>,
    ) -> Result<Verified, String> {
        assert_eq!(token_type, ID_TOKEN_TYPE);
        oidc(cfg, http, &sparq_lws_authn::parse_jwt(token)?, dpop).await
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
        let doc = json!({"id": s, SOLID_OIDC_ISSUER: {"id": op}, "service": [
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
    /// [`sparq_lws_authn::expansion_budget`], and a document past it is refused.
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

    /// Review finding: the parser held an expanded predicate for every open level of nesting
    /// before it yielded a triple, so a document of well under a megabyte nesting a hundred
    /// thousand property lists under a 64 KiB prefix took gigabytes before the expansion budget
    /// saw anything. Nesting is bounded before the parser runs.
    #[test]
    fn deeply_nested_identity_documents_are_refused_unparsed() {
        let s = "https://alice.example/id";
        let op = "https://op.example";
        let issuer = "<http://www.w3.org/ns/solid/terms#oidcIssuer>";
        let prefix = format!(
            "@prefix p: <https://p.example/{}#> .\n",
            "x".repeat(64 * 1024)
        );
        let depth = 100_000;
        let nested = format!(
            "{prefix}<{s}> {issuer} <{op}> .\n<{s}> p:p {}0{} .\n",
            "[ p:p ".repeat(depth),
            " ]".repeat(depth)
        );
        assert!(nested.len() < MAX_DOC);
        let started = std::time::Instant::now();
        assert_eq!(first_link("text/turtle", nested.as_bytes(), s, op), None);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // Collections nest the same way.
        let listed = format!(
            "{prefix}<{s}> {issuer} <{op}> .\n<{s}> p:p {}0{} .\n",
            "( ".repeat(depth),
            " )".repeat(depth)
        );
        assert_eq!(first_link("text/turtle", listed.as_bytes(), s, op), None);
        assert_eq!(
            first_link("application/n-triples", listed.as_bytes(), s, op),
            None
        );
        // A little nesting, and brackets inside IRIs, strings and comments, are fine.
        let shallow = format!(
            "{prefix}# ((((((((([[[[[[[[[\n<{s}> {issuer} <{op}> ; p:p [ p:p [ p:p \"[[[[[[[[[[\" ] ] ; \
             p:q \"\"\"(((((((((((\"\"\" ; p:r <urn:x:[[[[[[[[[[[> .\n"
        );
        assert_eq!(
            first_link("text/turtle", shallow.as_bytes(), s, op),
            Some(IssuerLink::SolidOidcIssuer)
        );
    }

    /// Review findings: escaped punctuation in prefixed names (`p:p\)`) hid nesting from a
    /// grammar-reading scan, and declarations under a long base held an expanded prefix each
    /// before any triple was yielded. The bound counts bytes, not tokens, so neither hides.
    #[test]
    fn turtle_parser_state_is_bounded_whatever_the_tokens() {
        let s = "https://alice.example/id";
        let op = "https://op.example";
        let issuer = "<http://www.w3.org/ns/solid/terms#oidcIssuer>";
        let long = "x".repeat(64 * 1024);
        let escaped = format!(
            "@prefix p: <https://p.example/{long}#> .\n<{s}> {issuer} <{op}> .\n<{s}> p:p {}0 .\n",
            "[ p:p\\) ".repeat(90_000)
        );
        let base = format!("@base <https://b.example/{long}/> .\n");
        let prefixes = format!(
            "{base}<{s}> {issuer} <{op}> .\n{}",
            (0..40_000)
                .map(|i| format!("@prefix p{i}: <> .\n"))
                .collect::<String>()
        );
        let shouting = prefixes
            .replace("@prefix", "PREFIX")
            .replace(" .\nPREFIX", "\nPREFIX");
        for doc in [&escaped, &prefixes, &shouting] {
            assert!(doc.len() < MAX_DOC);
            assert!(!turtle_state_bounded(doc.as_bytes(), 32));
            let started = std::time::Instant::now();
            assert_eq!(first_link("text/turtle", doc.as_bytes(), s, op), None);
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
        }
        // RDF 1.2 triple terms and annotations open levels too.
        for opener in ["<< ", "<<( ", "{| "] {
            assert!(
                !turtle_state_bounded(opener.repeat(100_000).as_bytes(), 32),
                "{opener}"
            );
        }
        // A long prefix used many times is held once by the parser, and its expansions are
        // charged to the expansion budget as they are yielded.
        let reused = format!(
            "@prefix p: <https://p.example/{long}#> .\n<{s}> {issuer} <{op}> .\n{}",
            "p:a p:b p:c .\n".repeat(60_000)
        );
        assert!(reused.len() < MAX_DOC);
        assert!(turtle_state_bounded(reused.as_bytes(), 32));
        let started = std::time::Instant::now();
        assert_eq!(first_link("text/turtle", reused.as_bytes(), s, op), None);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
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

    /// Review finding: an ID Token's expiry was checked before discovery and the key fetch, so
    /// one that expired while they were slow still verified. The suite reports the token's
    /// expiry, and the server (`Verified::still_valid`, just before it issues) refuses it.
    #[tokio::test]
    async fn an_id_token_that_expires_during_its_fetches_is_not_still_valid() {
        let op = jose::EcKey::generate("op-key");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let bob = format!("{base}/bob");
        let doc = json!({"id": bob,
            "service": [{"type": "OpenIdProvider", "serviceEndpoint": base}]});
        let discovery = json!({"issuer": base, "jwks_uri": format!("{base}/jwks")});
        let jwks = json!({"keys": [op.public_jwk()]});
        let app = axum::Router::new()
            .route(
                "/bob",
                axum::routing::get(move || async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    axum::Json(doc)
                }),
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
        let mut cfg = cfg();
        cfg.allow_insecure_fetch = true;
        let http = fetch_client(&cfg.policy()).unwrap();
        let now = jose::now_secs();
        // One second short of the skew past its expiry when the exchange begins.
        let exp = now - sparq_lws_authn::SKEW_SECS + 1;
        let token = op.sign_jws(
            serde_json::Map::new(),
            &json!({"iss": base, "sub": bob, "aud": [cfg.issuer()],
                "azp": "https://app.example/", "iat": now - 120, "exp": exp}),
        );
        let replay = DpopReplay::default();
        let ctx = DpopContext {
            proof: None,
            replay: &replay,
        };
        let verified = verify(&cfg, &http, &token, ID_TOKEN_TYPE, &ctx)
            .await
            .unwrap();
        assert_eq!(verified.expires, exp);
        assert!(verified.still_valid(now));
        assert!(!verified.still_valid(jose::now_secs()));
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
        let http = fetch_client(&cfg.policy()).unwrap();
        let client = jose::EcKey::generate("client");
        let htu = cfg.token_endpoint.to_string();
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
        let http = fetch_client(&cfg.policy()).unwrap();
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
        let htu = cfg.token_endpoint.to_string();
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
            "@context": CID_CONTEXT,
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
            "@context": CID_CONTEXT,
            "id": me,
            SOLID_OIDC_ISSUER: {"id": op},
        })
        .to_string();
        assert!(names_issuer("application/ld+json", known.as_bytes(), me, op).solid_oidc_issuer);
        // A short document whose inline context defines a long term, used many times, could
        // expand far past its size: no JSON-LD is expanded, so it names no issuer.
        let long = format!("https://e.example/{}", "x".repeat(8 * 1024));
        let mut amplified = serde_json::Map::new();
        amplified.insert("@context".into(), json!({"t": long}));
        amplified.insert("@id".into(), json!(me));
        amplified.insert(
            "t".into(),
            Value::Array((0..4096).map(|i| json!({"t": i})).collect()),
        );
        let amplified = Value::Object(amplified).to_string();
        assert_eq!(
            names_issuer("application/ld+json", amplified.as_bytes(), me, op),
            IssuerLinks::default()
        );
        // Review finding: with no context at all, a long subject was copied into every quad the
        // parser built before it yielded one. Never parsed now.
        let long_subject = json!({
            "@id": format!("https://e.example/{}", "s".repeat(512 * 1024)),
            "https://p.example/": vec![0; 100_000],
        });
        let body = long_subject.to_string();
        assert!(body.len() < MAX_DOC);
        assert_eq!(
            names_issuer("application/ld+json", body.as_bytes(), me, op),
            IssuerLinks::default()
        );
        // Review finding: a bare string under the Solid property is a literal, not the issuer.
        let literal = json!({
            "@context": CID_CONTEXT,
            "id": me,
            SOLID_OIDC_ISSUER: op,
        })
        .to_string();
        assert!(!names_issuer("application/ld+json", literal.as_bytes(), me, op).solid_oidc_issuer);
        assert_eq!(
            names_issuer("application/ld+json", body.as_bytes(), me, op),
            IssuerLinks::default()
        );
    }

    /// Review findings (rounds 6 and 7): general JSON-LD expansion of an identity document, which
    /// is read before any signature is checked, could not be bounded before it allocated. Only
    /// the compact forms are read; expanded and graph forms name no issuer, and the fetch asks
    /// for Turtle first.
    #[test]
    fn only_compact_json_ld_profiles_are_read() {
        let me = "https://alice.example/#me";
        let op = "https://op.example/";
        let expanded = json!([{
            "@id": me,
            SOLID_OIDC_ISSUER: [{"@id": op}],
        }])
        .to_string();
        assert_eq!(
            names_issuer("application/ld+json", expanded.as_bytes(), me, op),
            IssuerLinks::default()
        );
        let graph = json!({
            "@context": {"solid": "http://www.w3.org/ns/solid/terms#"},
            "@graph": [{"@id": me, "solid:oidcIssuer": {"@id": op}}],
        })
        .to_string();
        assert_eq!(
            names_issuer("application/ld+json", graph.as_bytes(), me, op),
            IssuerLinks::default()
        );
        let compact = json!({"@id": me, SOLID_OIDC_ISSUER: {"@id": op}}).to_string();
        assert!(names_issuer("application/ld+json", compact.as_bytes(), me, op).solid_oidc_issuer);
        let turtle = format!("<{me}> <{SOLID_OIDC_ISSUER}> <{op}> .");
        assert!(names_issuer("text/turtle", turtle.as_bytes(), me, op).solid_oidc_issuer);
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
}
