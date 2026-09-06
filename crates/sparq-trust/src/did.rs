//! # `did.rs` — pluggable DID resolution + issuer-key binding (`did:key` / `did:web`)
//!
//! Closes the **operator-asserted issuer-key** gap (`sq-pfae.3`, the live forgery
//! vector D′ of the design's §3.3): until now a trust rule named its issuer's
//! verifying key as raw `trust:issuerKey` hex supplied DIRECTLY by the pod operator,
//! so a wrong/forged key in the policy graph silently admitted forged credentials.
//! This module lets a rule instead name its issuer by **DID** (`trust:issuerDid`) and
//! resolves that DID to the SAME [`PublicKey`] the admission gate's signature check
//! already consumes ([`mod@crate::admit`]), via a **pluggable** [`DidResolver`].
//!
//! It is **OPT-IN**: the whole module sits behind the default-OFF `did` cargo feature,
//! so the lean default `sparq-trust` build (and the byte-identical default `sparq-solid`
//! build) gain nothing — strict additivity (design §2.2 G6) is preserved.
//!
//! ## Two methods, two trust models (be precise about which you get)
//!
//! - **`did:key`** ([`DidKeyResolver`]) — **self-certifying, offline, no network.** The
//!   key material is encoded IN the DID identifier (multibase `z`-base58btc over a
//!   multicodec-prefixed compressed point), so "resolving" is a local decode: the DID
//!   *is* the key. There is no document to fetch and nothing to trust beyond the DID
//!   string itself. What it buys over raw `trust:issuerKey` hex is a **stable,
//!   round-trippable, method-tagged identifier** that the same string everywhere refers
//!   to one key — NOT a stronger root of trust (whoever could put a forged hex key in the
//!   policy could equally put a forged `did:key`).
//! - **`did:web`** ([`DidWebResolver`]) — **document-fetched, network/host-rooted.** The
//!   DID maps to an `https://` URL (per the W3C `did:web` method) serving a DID document;
//!   the issuer key is read from that document's verification method. The root of trust is
//!   **whoever controls the host + its TLS** — a genuine improvement over an operator
//!   pasting hex, but only as strong as that host's control. The HTTP fetch is **not**
//!   performed by this crate (keeping the default build dependency-light): the caller
//!   supplies a [`DidDocumentFetcher`], so the resolver itself is offline-testable and the
//!   crate forces no HTTP client onto anyone. Wiring a real fetcher is the integration
//!   seam (see the module tests for an in-memory fetcher).
//!
//! ## Honest scope — what DID resolution does and does NOT fix (`sq-pfae.3`)
//!
//! Resolving the issuer key from a DID **narrows** forgery vector D′; it does not close
//! it, and it adds **no** privacy:
//!
//! - **`did:key` is not a trust anchor.** It is self-certifying — it proves the DID and
//!   the key agree, nothing about WHO holds the key. Trust still flows from whoever
//!   authored the (Control-gated) policy that names the DID.
//! - **`did:web` is only as strong as host/TLS control.** A host takeover or a
//!   mis-issued certificate substitutes the issuer key. There is no key-history /
//!   `did:web` deactivation handling here, and revocation stays the input-only
//!   `sq-tu4e` guard.
//! - **No privacy / unlinkability.** This binds an issuer key; it changes nothing about
//!   the in-the-clear admission of the credential value (the verifier still learns
//!   `age 25`, not "≥ 18"). The ZK estate remains research-grade and externally
//!   **UNAUDITED** (`sq-qhy4`); `sparq-mpc` is honest-majority semi-honest only.
//! - **Curve caveat — sparq's keys are Baby-JubJub, not Ed25519.** The shipped issuer
//!   signature is Poseidon2-Schnorr over Baby-JubJub ([`sparq_zk::sig`]), so the standard
//!   `did:key` Ed25519 multicodec (`0xed01`, the `z6Mk…` form) does **not** apply. This
//!   module uses an explicit **sparq-private** multicodec ([`MULTICODEC_BABYJUBJUB_PUB`])
//!   for the compressed Baby-JubJub point, and `did:web` documents must carry the sparq
//!   cryptosuite key. A DID minted for a different curve fails closed. This is an honest
//!   non-interop point with the wider `did:key` ecosystem, recorded so a working group can
//!   register a real codec.
//!
//! [OPUS-4.8] sq-pfae.3 (issue #940 / gh-940). Written while Fable unavailable — flag for
//! re-review when Fable returns. 🤖 SPARQ agent — trust-graph authorisation PoC.

use sparq_zk::sig::{public_key_from_hex, public_key_to_hex, PublicKey};

/// The multibase prefix for **base58btc** (`z`), the encoding `did:key` mandates for
/// its multicodec-prefixed key material (W3C `did:key` method §3.1).
pub const MULTIBASE_BASE58BTC: char = 'z';

/// A **sparq-private** multicodec code for a compressed Baby-JubJub public key, as an
/// unsigned-varint prefix on the 32-byte compressed point. The standard multicodec
/// table has no Baby-JubJub entry (sparq's curve is non-standard for `did:key`), so this
/// uses a value in the private-use space. It is documented as **non-interoperable**
/// with the wider `did:key` ecosystem and is the explicit curve-tag a working group would
/// replace with a registered code.
pub const MULTICODEC_BABYJUBJUB_PUB: u64 = 0x30_0100;

/// A parsed Decentralised Identifier: the method name and the method-specific id. Only
/// the syntactic shape is parsed here; method semantics live in the resolvers. Parsing
/// is fail-closed — a string that is not a syntactically valid `did:<method>:<id>`
/// yields `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Did {
    /// The DID method (e.g. `"key"`, `"web"`), lowercased per the spec's method-name rule.
    pub method: String,
    /// The method-specific identifier (everything after `did:<method>:`).
    pub method_specific_id: String,
}

impl Did {
    /// Parse a `did:<method>:<method-specific-id>` string. Fail-closed: returns `None`
    /// for anything not matching the generic DID syntax (missing `did:` scheme, empty
    /// method, empty id). The method name must be lowercase-or-digit (method names are
    /// case-insensitive per the DID-Core ABNF; an uppercase method is rejected here).
    pub fn parse(s: &str) -> Option<Did> {
        let rest = s.strip_prefix("did:")?;
        let (method, id) = rest.split_once(':')?;
        if method.is_empty() || id.is_empty() {
            return None;
        }
        // Method name is `1*method-char` of lowercase-or-digit; reject anything else so a
        // malformed method can never reach a resolver.
        if !method
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return None;
        }
        Some(Did {
            method: method.to_string(),
            method_specific_id: id.to_string(),
        })
    }

    /// Render the DID back to its canonical `did:<method>:<id>` string.
    pub fn to_did_string(&self) -> String {
        format!("did:{}:{}", self.method, self.method_specific_id)
    }
}

/// Why a DID could not be resolved to an issuer key. Every variant is a **fail-closed**
/// outcome: the admission gate admits nothing for a rule whose issuer DID does not
/// resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DidError {
    /// The string was not a syntactically valid DID.
    Malformed(String),
    /// The DID method is not supported by this resolver (only `key` / `web` are).
    UnsupportedMethod(String),
    /// A `did:key` identifier did not decode to a valid sparq issuer key (bad multibase,
    /// wrong multicodec, or not a valid compressed Baby-JubJub point).
    BadKeyMaterial(String),
    /// A `did:web` DID document could not be fetched (the fetcher returned an error).
    FetchFailed(String),
    /// A fetched `did:web` document had no usable sparq issuer verification method.
    NoIssuerKey(String),
}

impl std::fmt::Display for DidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Positional `format!` args (CodeQL rust/unused-variable false-positive avoidance).
            DidError::Malformed(s) => write!(f, "malformed DID: {}", s),
            DidError::UnsupportedMethod(m) => write!(f, "unsupported DID method: {}", m),
            DidError::BadKeyMaterial(s) => write!(f, "did:key material did not decode: {}", s),
            DidError::FetchFailed(s) => write!(f, "did:web document fetch failed: {}", s),
            DidError::NoIssuerKey(s) => {
                write!(f, "did:web document has no sparq issuer key: {}", s)
            }
        }
    }
}

impl std::error::Error for DidError {}

/// A pluggable DID resolver: map a DID to the issuer [`PublicKey`] the admission gate
/// verifies signatures against. Implementations MUST be fail-closed (return
/// `Err(DidError::…)` rather than a wrong key) so an unresolvable DID admits nothing.
pub trait DidResolver {
    /// Resolve a parsed DID to its issuer verifying key.
    fn resolve(&self, did: &Did) -> Result<PublicKey, DidError>;

    /// Convenience: parse `s` then [`resolve`](DidResolver::resolve). Returns
    /// [`DidError::Malformed`] if `s` is not a syntactically valid DID.
    fn resolve_str(&self, s: &str) -> Result<PublicKey, DidError> {
        let did = Did::parse(s).ok_or_else(|| DidError::Malformed(s.to_string()))?;
        self.resolve(&did)
    }
}

// --- did:key (offline, self-certifying) -----------------------------------

/// The offline `did:key` resolver: decode the multibase/multicodec key material in the
/// DID identifier to a [`PublicKey`]. No network, no document — the DID *is* the key.
///
/// Only the sparq Baby-JubJub multicodec ([`MULTICODEC_BABYJUBJUB_PUB`]) is accepted;
/// any other codec (e.g. the standard Ed25519 `z6Mk…`) fails closed with
/// [`DidError::BadKeyMaterial`], because sparq's issuer signature is Baby-JubJub.
#[derive(Debug, Clone, Copy, Default)]
pub struct DidKeyResolver;

impl DidResolver for DidKeyResolver {
    fn resolve(&self, did: &Did) -> Result<PublicKey, DidError> {
        if did.method != "key" {
            return Err(DidError::UnsupportedMethod(did.method.clone()));
        }
        decode_did_key_id(&did.method_specific_id)
    }
}

/// Mint the canonical `did:key` string for a sparq issuer [`PublicKey`]: multibase
/// (`z`-base58btc) over the multicodec-prefixed compressed Baby-JubJub point. Round-trips
/// with [`DidKeyResolver`]: `resolve(parse(did_key_for(pk))) == pk`.
pub fn did_key_for(pk: &PublicKey) -> String {
    // Compressed point bytes (the same 32-byte image `public_key_to_hex` encodes).
    let point = from_hex(&public_key_to_hex(pk)).expect("hex from public_key_to_hex is valid");
    let mut payload = encode_varint(MULTICODEC_BABYJUBJUB_PUB);
    payload.extend_from_slice(&point);
    let mb = base58btc_encode(&payload);
    format!("did:key:{}{}", MULTIBASE_BASE58BTC, mb)
}

/// Decode a `did:key` method-specific id to a [`PublicKey`]. Fail-closed on a wrong
/// multibase prefix, bad base58, the wrong multicodec, or a non-Baby-JubJub point.
fn decode_did_key_id(id: &str) -> Result<PublicKey, DidError> {
    let prefix = id
        .chars()
        .next()
        .ok_or_else(|| DidError::BadKeyMaterial("empty did:key id".to_string()))?;
    if prefix != MULTIBASE_BASE58BTC {
        return Err(DidError::BadKeyMaterial(format!(
            "unsupported multibase prefix `{}` (only `z`-base58btc)",
            prefix
        )));
    }
    let b58 = &id[prefix.len_utf8()..];
    let payload = base58btc_decode(b58)
        .ok_or_else(|| DidError::BadKeyMaterial(format!("bad base58: {}", id)))?;
    let (codec, rest) = decode_varint(&payload)
        .ok_or_else(|| DidError::BadKeyMaterial("truncated multicodec varint".to_string()))?;
    if codec != MULTICODEC_BABYJUBJUB_PUB {
        return Err(DidError::BadKeyMaterial(format!(
            "unsupported multicodec 0x{:x} (sparq uses Baby-JubJub 0x{:x})",
            codec, MULTICODEC_BABYJUBJUB_PUB
        )));
    }
    // The compressed point is the remaining bytes; `public_key_from_hex` re-checks it is
    // a valid (non-identity) Baby-JubJub point and fails closed otherwise.
    let hex = to_hex(rest);
    public_key_from_hex(&hex)
        .ok_or_else(|| DidError::BadKeyMaterial(format!("not a valid Baby-JubJub point: {}", id)))
}

// --- did:web (document-fetched, pluggable fetcher) ------------------------

/// A pluggable fetcher for a `did:web` DID document. The resolver hands it the resolved
/// `https://…/did.json` URL; the implementation returns the document bytes (UTF-8 JSON)
/// or an error. This is the **network seam** — the crate ships no HTTP client of its own,
/// so the default build forces no `reqwest`/`ureq` onto anyone, and the resolver stays
/// offline-testable (an in-memory map is a perfectly valid fetcher for tests / a
/// pre-cached registry).
pub trait DidDocumentFetcher {
    /// Fetch the DID document at `url` (always an `https://` URL ending in `did.json`).
    /// Return the raw UTF-8 JSON, or a human-readable error string (mapped to
    /// [`DidError::FetchFailed`]).
    fn fetch(&self, url: &str) -> Result<String, String>;
}

/// The `did:web` resolver over a pluggable [`DidDocumentFetcher`]. Maps the DID to its
/// `https://` document URL (W3C `did:web` §3.2), fetches it, and extracts the sparq
/// issuer key from the document's verification method.
pub struct DidWebResolver<F: DidDocumentFetcher> {
    fetcher: F,
}

impl<F: DidDocumentFetcher> DidWebResolver<F> {
    /// Build a `did:web` resolver over the given fetcher.
    pub fn new(fetcher: F) -> Self {
        DidWebResolver { fetcher }
    }

    /// Map a `did:web` method-specific id to its document URL per the method spec:
    /// the authority (with an optional `%3A`-encoded port) and `:`-separated path
    /// segments become a `/`-separated path. A bare-host DID (`did:web:example.com`)
    /// resolves to `https://example.com/.well-known/did.json`; a pathful DID
    /// (`did:web:example.com:user:alice`) resolves to
    /// `https://example.com/user/alice/did.json`. Returns `None` for an empty/invalid id.
    pub fn document_url(id: &str) -> Option<String> {
        if id.is_empty() {
            return None;
        }
        let mut parts = id.split(':');
        let authority = parts.next()?;
        if authority.is_empty() {
            return None;
        }
        // `%3A` in the authority is a port separator per the spec.
        let authority = authority.replace("%3A", ":");
        let path_segments: Vec<&str> = parts.collect();
        if path_segments.iter().any(|s| s.is_empty()) {
            return None; // a `::` empty path segment is malformed
        }
        if path_segments.is_empty() {
            Some(format!("https://{}/.well-known/did.json", authority))
        } else {
            Some(format!(
                "https://{}/{}/did.json",
                authority,
                path_segments.join("/")
            ))
        }
    }
}

impl<F: DidDocumentFetcher> DidResolver for DidWebResolver<F> {
    fn resolve(&self, did: &Did) -> Result<PublicKey, DidError> {
        if did.method != "web" {
            return Err(DidError::UnsupportedMethod(did.method.clone()));
        }
        let url = Self::document_url(&did.method_specific_id)
            .ok_or_else(|| DidError::Malformed(did.to_did_string()))?;
        let doc = self.fetcher.fetch(&url).map_err(DidError::FetchFailed)?;
        extract_issuer_key_from_doc(&doc, &did.to_did_string())
    }
}

/// Extract the sparq issuer [`PublicKey`] from a `did:web` DID document (JSON).
///
/// The document must carry a verification method whose `publicKeyMultibase` is the SAME
/// multibase/multicodec encoding `did:key` uses (so a `did:web` document and a `did:key`
/// agree on the curve tag), OR a `publicKeyHex` carrying the raw compressed point hex.
/// The first verification method that decodes to a valid sparq key wins; if none do, this
/// fails closed with [`DidError::NoIssuerKey`].
fn extract_issuer_key_from_doc(doc: &str, did: &str) -> Result<PublicKey, DidError> {
    let value: serde_json::Value =
        serde_json::from_str(doc).map_err(|e| DidError::NoIssuerKey(format!("{}: {}", did, e)))?;
    let methods = collect_verification_methods(&value);
    for m in methods {
        // `publicKeyMultibase` (preferred — same encoding as did:key).
        if let Some(mb) = m.get("publicKeyMultibase").and_then(|v| v.as_str()) {
            if let Ok(pk) = decode_did_key_id(mb) {
                return Ok(pk);
            }
        }
        // `publicKeyHex` (raw compressed Baby-JubJub point hex — sparq-zk's native form).
        if let Some(hex) = m.get("publicKeyHex").and_then(|v| v.as_str()) {
            if let Some(pk) = public_key_from_hex(hex) {
                return Ok(pk);
            }
        }
    }
    Err(DidError::NoIssuerKey(did.to_string()))
}

/// Collect candidate verification-method JSON objects from a DID document: the
/// embedded objects under `verificationMethod` plus any embedded under `assertionMethod`.
/// (A production resolver would also honour bare relationship references; the PoC scans
/// the embedded methods, which is sufficient for the issuer-key binding.)
fn collect_verification_methods(doc: &serde_json::Value) -> Vec<&serde_json::Value> {
    let mut out = Vec::new();
    for key in ["verificationMethod", "assertionMethod"] {
        if let Some(arr) = doc.get(key).and_then(|v| v.as_array()) {
            for m in arr {
                if m.is_object() {
                    out.push(m);
                }
            }
        }
    }
    out
}

// --- minimal multibase / multicodec / hex (no extra deps — lean-core rule) -----

/// The base58btc alphabet (Bitcoin/IPFS ordering), as used by multibase `z`.
const B58_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Encode bytes as base58btc (no multibase prefix). Big-endian, leading-zero preserving.
fn base58btc_encode(input: &[u8]) -> String {
    // Count leading zero bytes (each → a leading '1').
    let zeros = input.iter().take_while(|&&b| b == 0).count();
    let mut digits: Vec<u8> = Vec::new();
    for &byte in input {
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    for _ in 0..zeros {
        out.push('1');
    }
    for &d in digits.iter().rev() {
        out.push(B58_ALPHABET[d as usize] as char);
    }
    if out.is_empty() {
        out.push('1');
    }
    out
}

/// Decode a base58btc string (no multibase prefix). `None` on any non-alphabet char.
fn base58btc_decode(input: &str) -> Option<Vec<u8>> {
    if input.is_empty() {
        return None;
    }
    let mut bytes: Vec<u8> = Vec::new();
    for ch in input.chars() {
        let val = B58_ALPHABET.iter().position(|&c| c as char == ch)? as u32;
        let mut carry = val;
        for b in bytes.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Restore leading zero bytes (each leading '1' → a 0x00).
    let zeros = input.chars().take_while(|&c| c == '1').count();
    bytes.extend(std::iter::repeat_n(0u8, zeros));
    bytes.reverse();
    Some(bytes)
}

/// Encode an unsigned-LEB128 varint (the multicodec encoding).
fn encode_varint(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
    out
}

/// Decode an unsigned-LEB128 varint from the front of `bytes`; returns `(value, rest)`.
/// `None` on a truncated or over-long (> 64-bit) varint (fail-closed).
fn decode_varint(bytes: &[u8]) -> Option<(u64, &[u8])> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    for (i, &b) in bytes.iter().enumerate() {
        if shift >= 64 {
            return None; // over-long
        }
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((value, &bytes[i + 1..]));
        }
        shift += 7;
    }
    None // truncated
}

/// Lowercase-hex encode. `pub(crate)` so the `expression` module's
/// [`crate::expression::ChallengeNonce::generate`] renders its CSPRNG draw with
/// the same encoder rather than a second copy of it (`expression` implies `did`).
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Positional args (CodeQL rust/unused-variable false-positive avoidance).
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Decode lowercase/uppercase hex (optional `0x`), `None` on odd length / bad nibble.
fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_zk::sig::SecretKey;

    fn test_pk() -> PublicKey {
        // A deterministic non-identity issuer key for the round-trip tests.
        SecretKey::from_seed(0x1234_5678_9abc_def0).public_key()
    }

    #[test]
    fn did_parse_round_trips() {
        let d = Did::parse("did:key:zABC").unwrap();
        assert_eq!(d.method, "key");
        assert_eq!(d.method_specific_id, "zABC");
        assert_eq!(d.to_did_string(), "did:key:zABC");
        // Fail-closed shapes.
        assert!(Did::parse("did:key:").is_none());
        assert!(Did::parse("did::id").is_none());
        assert!(Did::parse("notadid").is_none());
        assert!(Did::parse("did:KEY:x").is_none()); // uppercase method rejected
    }

    #[test]
    fn base58_round_trips() {
        let cases: Vec<Vec<u8>> = vec![
            vec![0u8],
            vec![0, 0, 1, 2, 3],
            vec![255, 254, 0, 7],
            (0u8..40).collect(),
        ];
        for v in cases {
            let enc = base58btc_encode(&v);
            let dec = base58btc_decode(&enc).unwrap();
            assert_eq!(dec, v, "base58 round-trip for {:?}", v);
        }
        assert!(base58btc_decode("0OIl").is_none()); // chars not in the alphabet
    }

    #[test]
    fn varint_round_trips() {
        for v in [0u64, 1, 127, 128, 300, MULTICODEC_BABYJUBJUB_PUB, u64::MAX] {
            let enc = encode_varint(v);
            let (dec, rest) = decode_varint(&enc).unwrap();
            assert_eq!(dec, v);
            assert!(rest.is_empty());
        }
        assert!(decode_varint(&[0x80]).is_none()); // truncated
    }

    #[test]
    fn did_key_round_trips_to_the_same_public_key() {
        let pk = test_pk();
        let did_str = did_key_for(&pk);
        assert!(did_str.starts_with("did:key:z"));
        let resolver = DidKeyResolver;
        let resolved = resolver.resolve_str(&did_str).unwrap();
        assert_eq!(resolved, pk, "did:key resolves to the SAME issuer key");
    }

    #[test]
    fn did_key_fails_closed_on_wrong_method_and_bad_material() {
        let resolver = DidKeyResolver;
        // Wrong method.
        let web = Did::parse("did:web:example.com").unwrap();
        assert!(matches!(
            resolver.resolve(&web),
            Err(DidError::UnsupportedMethod(_))
        ));
        // Bad multibase prefix.
        assert!(matches!(
            resolver.resolve_str("did:key:QABC"),
            Err(DidError::BadKeyMaterial(_))
        ));
        // A valid base58 string but the wrong multicodec (Ed25519 0xed01 → not sparq).
        let mut payload = encode_varint(0xed01);
        payload.extend_from_slice(&[0u8; 32]);
        let wrong = format!("did:key:z{}", base58btc_encode(&payload));
        assert!(matches!(
            resolver.resolve_str(&wrong),
            Err(DidError::BadKeyMaterial(_))
        ));
    }

    #[test]
    fn did_web_document_url_mapping() {
        type R = DidWebResolver<NullFetcher>;
        assert_eq!(
            R::document_url("example.com"),
            Some("https://example.com/.well-known/did.json".to_string())
        );
        assert_eq!(
            R::document_url("example.com:user:alice"),
            Some("https://example.com/user/alice/did.json".to_string())
        );
        assert_eq!(
            R::document_url("example.com%3A3000:u"),
            Some("https://example.com:3000/u/did.json".to_string())
        );
        assert_eq!(R::document_url(""), None);
        assert_eq!(R::document_url("example.com::x"), None); // empty segment
    }

    /// A fetcher that always errors (for the URL-mapping type alias only).
    struct NullFetcher;
    impl DidDocumentFetcher for NullFetcher {
        fn fetch(&self, _url: &str) -> Result<String, String> {
            Err("null".to_string())
        }
    }

    #[cfg(feature = "did")]
    #[test]
    fn did_web_resolves_from_an_in_memory_document() {
        use std::collections::HashMap;

        let pk = test_pk();
        // Same multibase/multicodec encoding as did:key (strip the `did:key:` head).
        let mb = did_key_for(&pk)
            .strip_prefix("did:key:")
            .unwrap()
            .to_string();
        let did = "did:web:issuer.example:gov";
        let url = DidWebResolver::<MapFetcher>::document_url("issuer.example:gov").unwrap();
        let doc = format!(
            r#"{{
              "id": "{did}",
              "verificationMethod": [
                {{ "id": "{did}#k1", "type": "Multikey", "publicKeyMultibase": "{mb}" }}
              ],
              "assertionMethod": ["{did}#k1"]
            }}"#
        );
        let mut map = HashMap::new();
        map.insert(url, doc);
        let resolver = DidWebResolver::new(MapFetcher(map));
        let resolved = resolver.resolve_str(did).unwrap();
        assert_eq!(resolved, pk, "did:web resolves the document's issuer key");
    }

    #[cfg(feature = "did")]
    #[test]
    fn did_web_fails_closed_on_missing_or_keyless_document() {
        use std::collections::HashMap;
        // Empty registry → fetch fails → FetchFailed.
        let resolver = DidWebResolver::new(MapFetcher(HashMap::new()));
        assert!(matches!(
            resolver.resolve_str("did:web:issuer.example"),
            Err(DidError::FetchFailed(_))
        ));
        // Present but keyless document → NoIssuerKey.
        let url = DidWebResolver::<MapFetcher>::document_url("issuer.example").unwrap();
        let mut map = HashMap::new();
        map.insert(url, r#"{"id":"did:web:issuer.example"}"#.to_string());
        let resolver = DidWebResolver::new(MapFetcher(map));
        assert!(matches!(
            resolver.resolve_str("did:web:issuer.example"),
            Err(DidError::NoIssuerKey(_))
        ));
    }

    #[cfg(feature = "did")]
    struct MapFetcher(std::collections::HashMap<String, String>);
    #[cfg(feature = "did")]
    impl DidDocumentFetcher for MapFetcher {
        fn fetch(&self, url: &str) -> Result<String, String> {
            self.0
                .get(url)
                .cloned()
                .ok_or_else(|| format!("no document at {}", url))
        }
    }
}
