//! SAML 2.0 subject tokens (`lws10-authn-saml`): a `saml:Assertion`, base64url-encoded (RFC 8693
//! section 3), from an identity provider this server trusts out of band
//! (`SOLID_SERVER_LWS_SAML_IDPS_FILE`: entity id to a PEM certificate, PEM public key, or public
//! JWK).
//!
//! The assertion must carry an enveloped XML-DSig signature, the direct child of the assertion,
//! whose one reference is the assertion itself (`#` + its `ID`, which no other element shares), with
//! only the enveloped-signature and exclusive canonicalization transforms, a SHA-2 digest, and an
//! `rsa-sha256` or `ecdsa-sha256` signature under the issuer's configured key. Anything else is
//! refused rather than interpreted, which keeps signature wrapping out: the assertion that is read
//! is the document's root, and the signature covers all of it.
//!
//! Then, as the reference authorization server checks: `Conditions` with `NotBefore` and
//! `NotOnOrAfter` around now, every `AudienceRestriction` naming this authorization server and no
//! condition this server cannot evaluate, and a `Subject/NameID`, which is the LWS subject. Every
//! `SubjectConfirmation` must be bearer (holder-of-key and sender-vouches need a proof this
//! endpoint does not take), with an unexpired `SubjectConfirmationData` whose `Recipient`, which
//! is required, is the client. Subject and client must be URIs.
//!
//! The XML is parsed with quick-xml into a small tree; a DOCTYPE (and so any entity but the
//! predefined ones) or a processing instruction is refused, and exclusive XML canonicalization
//! 1.0 (without comments, with an optional `InclusiveNamespaces PrefixList`) is implemented here.

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use quick_xml::events::{BytesStart, Event};
use serde_json::Value;
use sha2::Digest;

use super::jose;
use super::subject_tokens::{parse_datetime, Verified, SKEW_SECS};
use super::{is_uri, LwsConfig};

pub const SAML_NS: &str = "urn:oasis:names:tc:SAML:2.0:assertion";
pub const DS_NS: &str = "http://www.w3.org/2000/09/xmldsig#";
const EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";
const ENVELOPED: &str = "http://www.w3.org/2000/09/xmldsig#enveloped-signature";
const RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";
const ECDSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// Largest decoded assertion accepted.
const MAX_ASSERTION: usize = 256 * 1024;
/// Deepest element nesting accepted.
const MAX_DEPTH: usize = 64;

/// Work a document may cost per byte of it, beside [`BUDGET_BASE`]: see [`Budget`].
const BUDGET_PER_BYTE: usize = 16;
/// Work any document may cost, beside [`BUDGET_PER_BYTE`] for each of its bytes.
const BUDGET_BASE: usize = 1 << 20;
/// The error of a document past its [`Budget`].
const OVER_BUDGET: &str = "the assertion is too costly to verify";
/// What an element, or an attribute, costs beside its bytes.
const NODE_OVERHEAD: usize = 32;

/// How many prefixes an exclusive canonicalization transform may render inclusively. Real
/// assertions name a handful; canonicalization weighs each at every element it renders.
const MAX_INCLUSIVE_PREFIXES: usize = 64;

/// What a copied binding costs beside its prefix and URI: the map entry and two string headers.
const BINDING_OVERHEAD: usize = 64;

/// What sorting `n` keys of at most `key` bytes may cost: about `n log n` comparisons, each
/// weighing up to a key's bytes.
fn sort_cost(n: usize, key: usize) -> usize {
    n.saturating_mul(n.max(2).ilog2() as usize + 1)
        .saturating_mul(key + 1)
}

/// What looking a `key`-byte key up in an ordered map of `n` entries may cost: a comparison of
/// its bytes at each level of the tree.
fn lookup_cost(key: usize, n: usize) -> usize {
    (key + 1).saturating_mul((n + 1).ilog2() as usize + 1)
}

/// The work one document may cost to verify, fixed by its size when it is parsed and charged by
/// everything that grows with what the document says rather than with its bytes: the tree the
/// parser builds (names, values, text, the namespace bindings an element copies) and both
/// canonicalizations (the bytes rendered and the prefixes weighed at each element). A document
/// past it is refused, whatever part of it is costly; no step has a bound of its own to miss.
struct Budget(std::cell::Cell<usize>);

impl Budget {
    fn for_document(len: usize) -> Self {
        Budget(std::cell::Cell::new(
            len.saturating_mul(BUDGET_PER_BYTE)
                .saturating_add(BUDGET_BASE),
        ))
    }

    fn charge(&self, n: usize) -> Result<(), String> {
        match self.0.get().checked_sub(n) {
            Some(left) => {
                self.0.set(left);
                Ok(())
            }
            None => {
                self.0.set(0);
                Err(OVER_BUDGET.into())
            }
        }
    }
}

/// A parsed document and what is left of its [`Budget`]: the only way to a tree, and the only way
/// to canonicalize one, so every step of a verification draws on the same budget.
pub struct Document {
    root: Element,
    budget: Budget,
}

impl Document {
    /// Parse `xml` into its root element; see [`parse_tree`].
    pub fn parse(xml: &str) -> Result<Self, String> {
        let budget = Budget::for_document(xml.len());
        let root = parse_tree(xml, &budget)?;
        Ok(Document { root, budget })
    }

    pub fn root(&self) -> &Element {
        &self.root
    }

    /// Exclusive XML Canonicalization 1.0 of the subtree at `el` (an element of this document);
    /// see [`render`].
    fn canonicalize(
        &self,
        el: &Element,
        exclude: Option<*const Element>,
        inclusive: &[String],
    ) -> Result<String, String> {
        let mut out = String::new();
        render(
            el,
            exclude,
            inclusive,
            &mut BTreeMap::new(),
            &self.budget,
            &mut out,
        )?;
        Ok(out)
    }
}

/// Verify a base64url-encoded SAML 2.0 assertion for an exchange at this authorization server.
pub fn verify(cfg: &LwsConfig, token: &str) -> Result<Verified, String> {
    verify_at(cfg, token, jose::now_secs())
}

pub fn verify_at(cfg: &LwsConfig, token: &str, now: i64) -> Result<Verified, String> {
    let compact: String = token.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = URL_SAFE_NO_PAD
        .decode(compact.trim_end_matches('='))
        .or_else(|_| STANDARD.decode(&compact))
        .map_err(|_| "the subject token is not a base64url-encoded XML document")?;
    if bytes.len() > MAX_ASSERTION {
        return Err("the assertion is too large".into());
    }
    let xml = std::str::from_utf8(&bytes).map_err(|_| "the assertion is not UTF-8")?;
    let doc = Document::parse(xml)?;
    let root = doc.root();
    if !root.is(SAML_NS, "Assertion") {
        return Err("the subject token is not a saml:Assertion".into());
    }
    if root.attr("Version") != Some("2.0") {
        return Err("the assertion is not SAML 2.0".into());
    }
    let issuer = root
        .child(SAML_NS, "Issuer")
        .map(|e| e.text().trim().to_string())
        .ok_or("the assertion names no Issuer")?;
    let trusted = cfg.saml_idps.get(&issuer).ok_or_else(|| {
        format!("the assertion's issuer {issuer} is not a trusted identity provider")
    })?;
    let key = TrustKey::parse(trusted)
        .map_err(|e| format!("the configured key of {issuer} is unusable: {e}"))?;
    verify_signature(&doc, &key)?;

    let conditions = root
        .child(SAML_NS, "Conditions")
        .ok_or("the assertion has no Conditions")?;
    let time = |name: &str| -> Result<i64, String> {
        conditions
            .attr(name)
            .and_then(parse_datetime)
            .ok_or_else(|| format!("Conditions {name} is missing or not an xsd:dateTime"))
    };
    if time("NotBefore")? > now + SKEW_SECS || time("NotOnOrAfter")? <= now - SKEW_SECS {
        return Err("the assertion is outside its validity period".into());
    }
    check_conditions(conditions, cfg.issuer())?;
    let subject = root
        .child(SAML_NS, "Subject")
        .ok_or("the assertion names no subject")?;
    let name_id = subject
        .child(SAML_NS, "NameID")
        .map(|e| e.text().trim().to_string())
        .unwrap_or_default();
    if name_id.is_empty() {
        return Err("the assertion names no subject".into());
    }
    // The NameID is the LWS subject, which an access token's sub carries: it MUST be a URI.
    if !is_uri(&name_id) {
        return Err("the assertion's NameID is not a URI".into());
    }
    let client = bearer_recipient(subject, now)?;
    Ok(Verified {
        subject: name_id,
        client,
    })
}

/// SAML 2.0 core 2.4.1.1: the bearer confirmation method, the one an assertion presented by
/// whoever holds it may use. Holder-of-key needs proof of a key and sender-vouches an
/// authenticated attesting party; this endpoint has neither, so an assertion confirmed that way
/// is refused rather than accepted without its proof.
const BEARER: &str = "urn:oasis:names:tc:SAML:2.0:cm:bearer";

/// Check the assertion's `Conditions` beyond its validity period (SAML 2.0 core 2.5.1):
/// - each `AudienceRestriction` is evaluated on its own, and every one of them must name this
///   authorization server ("the assertion is addressed to [...] each AudienceRestriction" — the
///   restrictions are ANDed, the audiences within one ORed);
/// - `ProxyRestriction` limits the assertions a relying party may issue on the strength of this
///   one; this server issues access tokens, never assertions, so it is satisfied and ignored;
/// - `OneTimeUse` is refused: honouring it needs a replay cache of assertions this server does not
///   keep, and an assertion whose issuer asked for single use must not be accepted repeatedly;
/// - any other condition (a `Condition` extension, or anything this server does not know) is
///   refused, since a relying party that cannot evaluate a condition must treat the assertion as
///   indeterminate (core 2.5.1.1), never as valid.
fn check_conditions(conditions: &Element, us: &str) -> Result<(), String> {
    let mut restrictions = 0;
    for condition in conditions.elements() {
        if condition.is(SAML_NS, "AudienceRestriction") {
            restrictions += 1;
            let named = condition
                .elements()
                .filter(|a| a.is(SAML_NS, "Audience"))
                .map(|a| a.text())
                .any(|a| a.trim() == us || a.trim().strip_suffix('/') == Some(us));
            if !named {
                return Err(
                    "an AudienceRestriction of the assertion does not include this authorization server"
                        .into(),
                );
            }
        } else if condition.is(SAML_NS, "ProxyRestriction") {
            continue;
        } else if condition.is(SAML_NS, "OneTimeUse") {
            return Err("the assertion is OneTimeUse, which this server cannot honour".into());
        } else {
            return Err(format!(
                "the assertion has a condition this server does not understand: {}",
                condition.qname()
            ));
        }
    }
    if restrictions == 0 {
        return Err("the assertion's audience does not include this authorization server".into());
    }
    Ok(())
}

/// The client an assertion was issued to: the `Recipient` of its bearer subject confirmation.
///
/// Every `SubjectConfirmation` must use the bearer method (holder-of-key, sender-vouches and
/// anything else are refused, not skipped), and there must be at least one. Each must carry
/// `SubjectConfirmationData` with a `NotOnOrAfter` in the future, a `NotBefore` (if any) in the
/// past, and a `Recipient` that is a URI: the LWS client identifier ("The SAML token MUST use the
/// Recipient parameter within a saml:SubjectConfirmationData assertion for the LWS client
/// identifier"). When several confirmations are present they must all name the same client.
fn bearer_recipient(subject: &Element, now: i64) -> Result<String, String> {
    let mut client: Option<String> = None;
    for confirmation in subject
        .elements()
        .filter(|e| e.is(SAML_NS, "SubjectConfirmation"))
    {
        if confirmation.attr("Method") != Some(BEARER) {
            return Err(format!(
                "the assertion's subject confirmation method {} is not accepted; only bearer is",
                confirmation.attr("Method").unwrap_or("(none)")
            ));
        }
        let data = confirmation
            .child(SAML_NS, "SubjectConfirmationData")
            .ok_or("the bearer subject confirmation has no SubjectConfirmationData")?;
        let limit = data
            .attr("NotOnOrAfter")
            .ok_or("the bearer subject confirmation has no NotOnOrAfter")?;
        if parse_datetime(limit).is_none_or(|t| t <= now - SKEW_SECS) {
            return Err("the subject confirmation has expired".into());
        }
        if let Some(start) = data.attr("NotBefore") {
            if parse_datetime(start).is_none_or(|t| t > now + SKEW_SECS) {
                return Err("the subject confirmation is not yet valid".into());
            }
        }
        // Without a Recipient the client is unknown.
        let recipient = data
            .attr("Recipient")
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .ok_or("the assertion's SubjectConfirmationData names no Recipient (the client)")?;
        if !is_uri(recipient) {
            return Err("the assertion's Recipient is not a URI".into());
        }
        match &client {
            Some(c) if c != recipient => {
                return Err(
                    "the assertion's subject confirmations name different Recipients".into(),
                )
            }
            _ => client = Some(recipient.to_string()),
        }
    }
    client.ok_or_else(|| "the assertion has no bearer SubjectConfirmation".into())
}

// ---------------------------------------------------------------- signature

fn verify_signature(doc: &Document, key: &TrustKey) -> Result<(), String> {
    let root = doc.root();
    let signatures: Vec<&Element> = root
        .elements()
        .filter(|e| e.is(DS_NS, "Signature"))
        .collect();
    let signature = match signatures.as_slice() {
        [] => return Err("the assertion is not signed".into()),
        [one] => *one,
        _ => return Err("the assertion carries more than one signature".into()),
    };
    let id = root
        .attr("ID")
        .filter(|i| !i.is_empty())
        .ok_or("the assertion has no ID")?;
    let mut same_id = 0;
    root.count_ids(id, &mut same_id);
    if same_id != 1 {
        return Err("the assertion's ID is not unique in the document".into());
    }
    let signed_info = signature
        .child(DS_NS, "SignedInfo")
        .ok_or("the signature has no SignedInfo")?;
    let c14n = signed_info
        .child(DS_NS, "CanonicalizationMethod")
        .ok_or("SignedInfo names no CanonicalizationMethod")?;
    if c14n.attr("Algorithm") != Some(EXC_C14N) {
        return Err("SignedInfo is not canonicalized with exclusive XML canonicalization".into());
    }
    let method = signed_info
        .child(DS_NS, "SignatureMethod")
        .and_then(|m| m.attr("Algorithm"))
        .ok_or("SignedInfo names no SignatureMethod")?;
    let references: Vec<&Element> = signed_info
        .elements()
        .filter(|e| e.is(DS_NS, "Reference"))
        .collect();
    let [reference] = references.as_slice() else {
        return Err("the signature must have exactly one reference".into());
    };
    if reference.attr("URI") != Some(format!("#{id}").as_str()) {
        return Err("the signature does not reference the assertion".into());
    }
    let mut enveloped = false;
    let mut exclusive = None;
    if let Some(transforms) = reference.child(DS_NS, "Transforms") {
        for t in transforms.elements() {
            match (t.is(DS_NS, "Transform"), t.attr("Algorithm")) {
                (true, Some(ENVELOPED)) if !enveloped && exclusive.is_none() => enveloped = true,
                (true, Some(EXC_C14N)) if exclusive.is_none() => {
                    exclusive = Some(inclusive_prefixes(t)?)
                }
                _ => return Err("the reference has a transform this server does not accept".into()),
            }
        }
    }
    let (true, Some(prefixes)) = (enveloped, exclusive) else {
        return Err("the reference must apply the enveloped-signature and exclusive canonicalization transforms".into());
    };
    let digest_alg = reference
        .child(DS_NS, "DigestMethod")
        .and_then(|d| d.attr("Algorithm"))
        .ok_or("the reference names no DigestMethod")?;
    let expected = reference
        .child(DS_NS, "DigestValue")
        .map(|d| d.text())
        .ok_or("the reference has no DigestValue")?;
    let expected = b64_decode(&expected).ok_or("the DigestValue is not base64")?;
    let canonical = doc.canonicalize(root, Some(signature as *const Element), &prefixes)?;
    let actual: Vec<u8> = match digest_alg {
        "http://www.w3.org/2001/04/xmlenc#sha256" => {
            sha2::Sha256::digest(canonical.as_bytes()).to_vec()
        }
        "http://www.w3.org/2001/04/xmldsig-more#sha384" => {
            sha2::Sha384::digest(canonical.as_bytes()).to_vec()
        }
        "http://www.w3.org/2001/04/xmlenc#sha512" => {
            sha2::Sha512::digest(canonical.as_bytes()).to_vec()
        }
        other => return Err(format!("unsupported digest {other}")),
    };
    if actual != expected {
        return Err("the assertion's digest does not match: it was altered after signing".into());
    }
    let signed = doc.canonicalize(signed_info, None, &inclusive_prefixes(c14n)?)?;
    let value = signature
        .child(DS_NS, "SignatureValue")
        .map(|v| v.text())
        .ok_or("the signature has no SignatureValue")?;
    let value = b64_decode(&value).ok_or("the SignatureValue is not base64")?;
    if !key.verify(method, signed.as_bytes(), &value)? {
        return Err("the assertion's signature does not verify".into());
    }
    Ok(())
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let compact: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    STANDARD.decode(compact).ok()
}

/// The `InclusiveNamespaces PrefixList` of an exclusive canonicalization method or transform.
fn inclusive_prefixes(method: &Element) -> Result<Vec<String>, String> {
    let mut prefixes: Vec<String> = method
        .elements()
        .find(|e| e.is(EXC_C14N, "InclusiveNamespaces"))
        .and_then(|e| e.attr("PrefixList"))
        .map(|l| {
            l.split_ascii_whitespace()
                .map(|p| {
                    if p == "#default" {
                        String::new()
                    } else {
                        p.to_string()
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    prefixes.sort();
    prefixes.dedup();
    // Canonicalization weighs every inclusive prefix at every element it renders.
    if prefixes.len() > MAX_INCLUSIVE_PREFIXES {
        return Err("the transform names too many inclusive namespace prefixes".into());
    }
    Ok(prefixes)
}

/// A trusted identity provider's signing key.
#[derive(Debug, Clone)]
pub enum TrustKey {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    P256(p256::PublicKey),
}

impl TrustKey {
    /// From a PEM certificate, a PEM public key (SPKI or PKCS#1 RSA), or a public JWK.
    pub fn parse(config: &str) -> Result<Self, String> {
        let config = config.trim();
        if config.starts_with('{') {
            let jwk: Value = serde_json::from_str(config).map_err(|_| "not a JWK")?;
            return match jwk.get("kty").and_then(Value::as_str) {
                Some("RSA") => {
                    let get = |k: &str| {
                        jwk.get(k)
                            .and_then(Value::as_str)
                            .and_then(jose::b64url_decode)
                            .ok_or("an RSA JWK needs n and e")
                    };
                    Ok(TrustKey::Rsa {
                        n: get("n")?,
                        e: get("e")?,
                    })
                }
                Some("EC") => jose::ec_public_from_jwk(&jwk)
                    .map(TrustKey::P256)
                    .ok_or_else(|| "not a P-256 JWK".into()),
                _ => Err("unsupported JWK key type".into()),
            };
        }
        let (label, der) = pem(config).ok_or("not PEM")?;
        match label.as_str() {
            "CERTIFICATE" => spki_key(certificate_spki(&der).ok_or("not an X.509 certificate")?),
            "PUBLIC KEY" => spki_key(&der),
            "RSA PUBLIC KEY" => rsa_public_key(&der),
            other => Err(format!("unsupported PEM {other}")),
        }
    }

    fn verify(&self, method: &str, message: &[u8], signature: &[u8]) -> Result<bool, String> {
        match (method, self) {
            (RSA_SHA256, TrustKey::Rsa { n, e }) => {
                let key = aws_lc_rs::signature::RsaPublicKeyComponents {
                    n: n.as_slice(),
                    e: e.as_slice(),
                };
                Ok(key
                    .verify(
                        &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA256,
                        message,
                        signature,
                    )
                    .is_ok())
            }
            (ECDSA_SHA256, TrustKey::P256(k)) => {
                use p256::ecdsa::signature::Verifier;
                let Ok(sig) = p256::ecdsa::Signature::from_slice(signature) else {
                    return Ok(false);
                };
                Ok(p256::ecdsa::VerifyingKey::from(k)
                    .verify(message, &sig)
                    .is_ok())
            }
            (m, _) => Err(format!(
                "signature method {m} does not fit the identity provider's key"
            )),
        }
    }
}

/// The label and DER of the first PEM block.
fn pem(text: &str) -> Option<(String, Vec<u8>)> {
    let start = text.find("-----BEGIN ")?;
    let rest = &text[start + 11..];
    let label_end = rest.find("-----")?;
    let label = rest[..label_end].to_string();
    let body_start = label_end + 5;
    let end = rest.find(&format!("-----END {label}-----"))?;
    let body: String = rest[body_start..end]
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    Some((label, STANDARD.decode(body).ok()?))
}

/// One DER TLV: tag, contents, and what follows.
fn der(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        (
            rest[..n]
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | *b as usize),
            &rest[n..],
        )
    };
    if rest.len() < len {
        return None;
    }
    Some((tag, &rest[..len], &rest[len..]))
}

/// The SubjectPublicKeyInfo of an X.509 certificate.
fn certificate_spki(cert: &[u8]) -> Option<&[u8]> {
    let (0x30, cert, _) = der(cert)? else {
        return None;
    };
    let (0x30, tbs, _) = der(cert)? else {
        return None;
    };
    let mut rest = tbs;
    let (tag, _, after) = der(rest)?;
    if tag == 0xa0 {
        rest = after; // explicit version
    }
    // serial, signature algorithm, issuer, validity, subject
    for _ in 0..5 {
        rest = der(rest)?.2;
    }
    let (0x30, _, after) = der(rest)? else {
        return None;
    };
    Some(&rest[..rest.len() - after.len()])
}

const RSA_OID: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const EC_OID: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const P256_OID: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];

fn spki_key(spki: &[u8]) -> Result<TrustKey, String> {
    let bad = || "not a SubjectPublicKeyInfo".to_string();
    let (0x30, body, _) = der(spki).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x30, alg, rest) = der(body).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x06, oid, params) = der(alg).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x03, bits, _) = der(rest).ok_or_else(bad)? else {
        return Err(bad());
    };
    let key = bits.strip_prefix(&[0u8]).ok_or_else(bad)?;
    if oid == RSA_OID {
        rsa_public_key(key)
    } else if oid == EC_OID {
        match der(params) {
            Some((0x06, curve, _)) if curve == P256_OID => jose::ec_public_from_sec1(key)
                .map(TrustKey::P256)
                .ok_or_else(|| "not a P-256 point".into()),
            _ => Err("only P-256 EC keys are supported".into()),
        }
    } else {
        Err("only RSA and P-256 keys are supported".into())
    }
}

fn rsa_public_key(pkcs1: &[u8]) -> Result<TrustKey, String> {
    let bad = || "not an RSA public key".to_string();
    let (0x30, body, _) = der(pkcs1).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x02, n, rest) = der(body).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x02, e, _) = der(rest).ok_or_else(bad)? else {
        return Err(bad());
    };
    let strip = |v: &[u8]| {
        let i = v.iter().position(|b| *b != 0).unwrap_or(v.len());
        v[i..].to_vec()
    };
    Ok(TrustKey::Rsa {
        n: strip(n),
        e: strip(e),
    })
}

// ---------------------------------------------------------------- XML tree

/// An element: its qualified name, resolved namespace, attributes, the namespaces in scope, and
/// children.
#[derive(Debug, Clone)]
pub struct Element {
    prefix: String,
    local: String,
    ns: String,
    attrs: Vec<Attr>,
    /// Every namespace in scope here, prefix (`""` for the default) to URI; `""` URI undeclares.
    scope: Arc<BTreeMap<String, String>>,
    children: Vec<Node>,
}

#[derive(Debug, Clone)]
struct Attr {
    prefix: String,
    local: String,
    ns: String,
    value: String,
}

#[derive(Debug, Clone)]
enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    fn is(&self, ns: &str, local: &str) -> bool {
        self.ns == ns && self.local == local
    }

    fn qname(&self) -> String {
        if self.prefix.is_empty() {
            self.local.clone()
        } else {
            format!("{}:{}", self.prefix, self.local)
        }
    }

    /// An unqualified attribute's value.
    fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.is_empty() && a.local == local)
            .map(|a| a.value.as_str())
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The first child element `{ns}local`.
    fn child(&self, ns: &str, local: &str) -> Option<&Element> {
        self.elements().find(|e| e.is(ns, local))
    }

    /// How many elements in this subtree carry an `ID`, `Id` or `AssertionID` of `id`.
    fn count_ids(&self, id: &str, count: &mut usize) {
        if self.attrs.iter().any(|a| {
            a.ns.is_empty()
                && matches!(a.local.as_str(), "ID" | "Id" | "AssertionID")
                && a.value == id
        }) {
            *count += 1;
        }
        for e in self.elements() {
            e.count_ids(id, count);
        }
    }

    /// The concatenated text content.
    fn text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }

    fn collect_text(&self, out: &mut String) {
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => e.collect_text(out),
            }
        }
    }
}

/// Resolve XML character and predefined entity references; normalize line ends. In an attribute
/// value, literal whitespace becomes a space (XML 1.0 section 3.3.3).
fn unescape(raw: &str, attribute: bool) -> Result<String, String> {
    let raw = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw.as_str();
    while let Some(i) = rest.find(['&', '\t', '\n']) {
        let (before, after) = rest.split_at(i);
        out.push_str(before);
        if !after.starts_with('&') {
            out.push(if attribute {
                ' '
            } else {
                after.chars().next().unwrap_or(' ')
            });
            rest = &after[1..];
            continue;
        }
        let end = after.find(';').ok_or("an unterminated reference")?;
        let name = &after[1..end];
        match name {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            n if n.starts_with("#x") => out.push(
                u32::from_str_radix(&n[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or("a bad character reference")?,
            ),
            n if n.starts_with('#') => out.push(
                n[1..]
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or("a bad character reference")?,
            ),
            _ => return Err(format!("the entity &{name}; is not allowed")),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn split_qname(q: &str) -> (String, String) {
    match q.split_once(':') {
        Some((p, l)) => (p.to_string(), l.to_string()),
        None => (String::new(), q.to_string()),
    }
}

fn start_element(
    e: &BytesStart<'_>,
    parent_scope: &Arc<BTreeMap<String, String>>,
    budget: &Budget,
) -> Result<Element, String> {
    let name = e.name();
    let qname = std::str::from_utf8(name.as_ref()).map_err(|_| "a non-UTF-8 name")?;
    budget.charge(qname.len() + NODE_OVERHEAD)?;
    let qname = qname.to_string();
    let mut declared = Vec::new();
    let mut raw_attrs = Vec::new();
    // Repeated names are found with a set: quick-xml's own check compares each attribute with
    // every one before it, which an element with tens of thousands of attributes makes quadratic.
    let mut names = std::collections::HashSet::new();
    for a in e.attributes().with_checks(false) {
        let a = a.map_err(|e| format!("malformed attribute: {e}"))?;
        let key = std::str::from_utf8(a.key.as_ref())
            .map_err(|_| "a non-UTF-8 attribute name")?
            .to_string();
        if !names.insert(key.clone()) {
            return Err("malformed attribute: a repeated attribute".into());
        }
        let raw = std::str::from_utf8(&a.value).map_err(|_| "a non-UTF-8 attribute value")?;
        budget.charge(key.len() + raw.len() + NODE_OVERHEAD)?;
        let value = unescape(raw, true)?;
        // Canonical XML 1.0 section 2.1, which exclusive canonicalization inherits: a document
        // with a relative namespace URI, even an unused one, cannot be canonicalized.
        let absolute = |v: &str| {
            v.split_once(':').is_some_and(|(scheme, _)| {
                scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                    && scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            })
        };
        if key == "xmlns" {
            if !value.is_empty() && !absolute(&value) {
                return Err("a relative namespace URI cannot be canonicalized".into());
            }
            declared.push((String::new(), value));
        } else if let Some(p) = key.strip_prefix("xmlns:") {
            if value.is_empty() {
                return Err("a prefixed namespace cannot be undeclared in XML 1.0".into());
            }
            if !absolute(&value) {
                return Err("a relative namespace URI cannot be canonicalized".into());
            }
            declared.push((p.to_string(), value));
        } else {
            raw_attrs.push((key, value));
        }
    }
    let scope = if declared.is_empty() {
        parent_scope.clone()
    } else {
        let bytes = |(k, v): (&String, &String)| k.len() + v.len() + BINDING_OVERHEAD;
        let copied = parent_scope.iter().map(bytes).sum::<usize>()
            + declared.iter().map(|(k, v)| bytes((k, v))).sum::<usize>();
        budget.charge(copied)?;
        let mut scope = BTreeMap::clone(parent_scope);
        scope.extend(declared);
        Arc::new(scope)
    };
    // Each element and attribute keeps its own copy of its namespace URI, charged before it is
    // made: a long URI used by many elements costs what its copies do.
    let owned = |uri: &str| -> Result<String, String> {
        budget.charge(uri.len())?;
        Ok(uri.to_string())
    };
    let (prefix, local) = split_qname(&qname);
    let ns = if prefix == "xml" {
        XML_NS.to_string()
    } else if prefix.is_empty() {
        owned(scope.get("").map(String::as_str).unwrap_or_default())?
    } else {
        let uri = scope
            .get(&prefix)
            .filter(|u| !u.is_empty())
            .ok_or_else(|| format!("unbound prefix {prefix}"))?;
        owned(uri)?
    };
    let mut attrs = Vec::new();
    let mut expanded = std::collections::HashSet::new();
    for (key, value) in raw_attrs {
        let (p, l) = split_qname(&key);
        let ans = if p.is_empty() {
            String::new()
        } else if p == "xml" {
            XML_NS.to_string()
        } else {
            let uri = scope
                .get(&p)
                .filter(|u| !u.is_empty())
                .ok_or_else(|| format!("unbound prefix {p}"))?;
            owned(uri)?
        };
        if !expanded.insert((ans.clone(), l.clone())) {
            return Err("a repeated attribute".into());
        }
        attrs.push(Attr {
            prefix: p,
            local: l,
            ns: ans,
            value,
        });
    }
    Ok(Element {
        prefix,
        local,
        ns,
        attrs,
        scope,
        children: Vec::new(),
    })
}

/// Parse a document into its root element, charging `budget` for what the tree holds. Comments
/// are dropped; a DOCTYPE, a processing instruction, or anything after the root is refused.
fn parse_tree(xml: &str, budget: &Budget) -> Result<Element, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let empty = Arc::new(BTreeMap::new());
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("not well-formed XML: {e}"))?;
        match event {
            Event::Start(e) | Event::Empty(e) if root.is_some() => {
                let _ = e;
                return Err("content after the root element".into());
            }
            Event::Start(e) => {
                if stack.len() >= MAX_DEPTH {
                    return Err("the XML nests too deeply".into());
                }
                let el =
                    start_element(&e, stack.last().map(|p| &p.scope).unwrap_or(&empty), budget)?;
                stack.push(el);
            }
            Event::Empty(e) => {
                let el =
                    start_element(&e, stack.last().map(|p| &p.scope).unwrap_or(&empty), budget)?;
                match stack.last_mut() {
                    Some(p) => p.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::End(_) => {
                let el = stack.pop().ok_or("an unmatched end tag")?;
                match stack.last_mut() {
                    Some(p) => p.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::Text(t) => {
                let raw = std::str::from_utf8(t.as_ref()).map_err(|_| "non-UTF-8 text")?;
                budget.charge(raw.len() + NODE_OVERHEAD)?;
                match stack.last_mut() {
                    Some(p) => p.children.push(Node::Text(unescape(raw, false)?)),
                    None if raw.trim().is_empty() => {}
                    None => return Err("text outside the root element".into()),
                }
            }
            Event::CData(t) => {
                let raw = std::str::from_utf8(t.as_ref()).map_err(|_| "non-UTF-8 text")?;
                budget.charge(raw.len() + NODE_OVERHEAD)?;
                let p = stack.last_mut().ok_or("text outside the root element")?;
                p.children
                    .push(Node::Text(raw.replace("\r\n", "\n").replace('\r', "\n")));
            }
            Event::Comment(_) | Event::Decl(_) => {}
            Event::DocType(_) => return Err("a DOCTYPE is not allowed".into()),
            Event::PI(_) => return Err("processing instructions are not allowed".into()),
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err("an unclosed element".into());
    }
    root.ok_or_else(|| "no root element".into())
}

// ---------------------------------------------------------------- exclusive canonicalization

/// How many bytes [`escape_text`] (or, for `attr`, [`escape_attr`]) writes for `s`, so the
/// bytes are charged before they are written.
fn escaped_len(s: &str, attr: bool) -> usize {
    s.chars()
        .map(|c| match c {
            '&' | '<' => 4 + usize::from(c == '&'),
            '>' if !attr => 4,
            '"' if attr => 6,
            '\t' | '\n' if attr => 5,
            '\r' => 5,
            c => c.len_utf8(),
        })
        .sum()
}

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
}

/// Exclusive XML Canonicalization 1.0 (without comments) of the subtree at `el`, leaving out the
/// element `exclude` (the enveloped signature). `rendered` holds the namespace declarations the
/// output ancestors rendered, one map that an element adds its declarations to for its
/// descendants and takes them back from after, so no element copies it; `inclusive` the
/// `InclusiveNamespaces` prefixes (`""` is the default namespace), which are rendered like
/// inclusive canonicalization does. Every element charges `budget` before each step it takes, by
/// what the step can weigh: the prefixes it looks up, compares and copies, the keys it sorts, the
/// bindings it records, and the bytes it writes.
fn render(
    el: &Element,
    exclude: Option<*const Element>,
    inclusive: &[String],
    rendered: &mut BTreeMap<String, String>,
    budget: &Budget,
    out: &mut String,
) -> Result<(), String> {
    if exclude == Some(el as *const Element) {
        return Ok(());
    }
    // Each inclusive prefix is looked up in scope and copied at every element, and each prefix
    // an attribute names is copied: all charged before it is done, by the bytes it weighs.
    let scope_len = el.scope.len();
    let inclusive_bytes: usize = inclusive
        .iter()
        .map(|p| lookup_cost(p.len(), scope_len) + p.len())
        .sum();
    let attr_prefixes: usize = el.attrs.iter().map(|a| a.prefix.len()).sum();
    budget.charge(
        (1 + el.attrs.len() + inclusive.len()) * NODE_OVERHEAD
            + inclusive_bytes
            + attr_prefixes
            + el.prefix.len(),
    )?;
    // Namespaces this element visibly utilizes, plus the inclusive ones in scope.
    let mut wanted: Vec<String> = vec![el.prefix.clone()];
    for a in &el.attrs {
        if !a.prefix.is_empty() && a.prefix != "xml" {
            wanted.push(a.prefix.clone());
        }
    }
    // An inclusive prefix is handled as inclusive canonicalization handles it: rendered when in
    // scope and not already rendered by an output ancestor. For the default namespace that
    // includes an undeclaration (`xmlns=""`) when an output ancestor rendered a non-empty default
    // (Exclusive XML Canonicalization 1.0, section 3; Canonical XML 1.0, section 2.3).
    for p in inclusive {
        if p.is_empty() || el.scope.get(p).is_some_and(|u| !u.is_empty()) {
            wanted.push(p.clone());
        }
    }
    let longest = wanted.iter().map(String::len).max().unwrap_or(0);
    budget.charge(sort_cost(wanted.len(), longest))?;
    wanted.sort();
    wanted.dedup();
    let mut decls = Vec::new();
    for p in wanted {
        if p == "xml" {
            continue;
        }
        // Looked up in scope and among the rendered, then its URI compared and copied.
        budget.charge(lookup_cost(p.len(), scope_len) + lookup_cost(p.len(), rendered.len()))?;
        let uri = el.scope.get(&p).map(String::as_str).unwrap_or_default();
        budget.charge(2 * uri.len())?;
        if p.is_empty() {
            let before = rendered.get("").map(String::as_str).unwrap_or("");
            if uri != before {
                decls.push((p, uri.to_string()));
            }
        } else if rendered.get(&p).map(String::as_str) != Some(uri) {
            if uri.is_empty() {
                return Err(format!("unbound prefix {p}"));
            }
            decls.push((p, uri.to_string()));
        }
    }
    let mut attrs: Vec<&Attr> = el.attrs.iter().collect();
    let longest = attrs
        .iter()
        .map(|a| a.ns.len() + a.local.len())
        .max()
        .unwrap_or(0);
    budget.charge(sort_cost(attrs.len(), longest))?;
    attrs.sort_by(|a, b| (a.ns.as_str(), a.local.as_str()).cmp(&(b.ns.as_str(), b.local.as_str())));
    let qname = el.qname();
    let tag = 2
        + qname.len()
        + decls
            .iter()
            .map(|(p, uri)| 10 + p.len() + escaped_len(uri, true))
            .sum::<usize>()
        + attrs
            .iter()
            .map(|a| 5 + a.prefix.len() + a.local.len() + escaped_len(&a.value, true))
            .sum::<usize>();
    budget.charge(tag)?;
    out.push('<');
    out.push_str(&qname);
    for (p, uri) in &decls {
        if p.is_empty() {
            out.push_str(" xmlns=\"");
        } else {
            out.push_str(" xmlns:");
            out.push_str(p);
            out.push_str("=\"");
        }
        escape_attr(uri, out);
        out.push('"');
    }
    for a in attrs {
        out.push(' ');
        if !a.prefix.is_empty() {
            out.push_str(&a.prefix);
            out.push(':');
        }
        out.push_str(&a.local);
        out.push_str("=\"");
        escape_attr(&a.value, out);
        out.push('"');
    }
    out.push('>');
    // What the descendants see as rendered, taken back once they are written: each binding
    // recorded and taken back costs two lookups.
    budget.charge(
        decls
            .iter()
            .map(|(p, _)| 2 * lookup_cost(p.len(), rendered.len() + decls.len()))
            .sum(),
    )?;
    let before: Vec<(String, Option<String>)> = decls
        .into_iter()
        .map(|(p, uri)| (p.clone(), rendered.insert(p, uri)))
        .collect();
    let children = el.children.iter().try_for_each(|c| match c {
        Node::Text(t) => {
            budget.charge(escaped_len(t, false))?;
            escape_text(t, out);
            Ok(())
        }
        Node::Element(e) => render(e, exclude, inclusive, rendered, budget, out),
    });
    for (p, old) in before.into_iter().rev() {
        match old {
            Some(uri) => rendered.insert(p, uri),
            None => rendered.remove(&p),
        };
    }
    children?;
    budget.charge(qname.len() + 3)?;
    out.push_str("</");
    out.push_str(&qname);
    out.push('>');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> Result<Element, String> {
        Document::parse(xml).map(|d| d.root)
    }

    fn c14n(xml: &str) -> String {
        let doc = Document::parse(xml).unwrap();
        doc.canonicalize(doc.root(), None, &[]).unwrap()
    }

    /// Review finding: every element copied every namespace binding in scope. An unsigned
    /// assertion with thousands of declarations on its root and tens of thousands of small
    /// children, well inside the size limit, held over a hundred million bindings before any
    /// signature check. Elements that declare nothing now share their parent's bindings, and the
    /// copies the declaring ones make are bounded.
    #[test]
    fn namespace_scopes_are_shared_and_bounded() {
        let decls: String = (0..4096)
            .map(|i| format!(" xmlns:p{i}=\"urn:{i}\""))
            .collect();
        let shared = format!("<r{decls}>{}</r>", "<a/>".repeat(30_000));
        let root = parse(&shared).unwrap();
        assert_eq!(root.children.len(), 30_000);
        assert!(root.children.iter().all(|c| match c {
            Node::Element(e) => Arc::ptr_eq(&e.scope, &root.scope),
            Node::Text(_) => false,
        }));
        let copied = format!("<r{decls}>{}</r>", "<a xmlns:q=\"urn:q\"/>".repeat(30_000));
        assert_eq!(parse(&copied).unwrap_err(), OVER_BUDGET);
        // A few bindings with long URIs cost what their bytes do.
        let long = format!("<r xmlns:p=\"urn:{}\">", "x".repeat(128 * 1024));
        let copied = format!("{long}{}</r>", "<a xmlns:q=\"urn:q\"/>".repeat(6_000));
        assert_eq!(parse(&copied).unwrap_err(), OVER_BUDGET);
        // So does a long URI that undeclaring descendants resolve to.
        let long = format!("<r xmlns=\"urn:{}\">", "x".repeat(128 * 1024));
        let used = format!("{long}{}</r>", "<a/>".repeat(30_000));
        assert_eq!(parse(&used).unwrap_err(), OVER_BUDGET);
        let long = format!("<r xmlns:p=\"urn:{}\">", "x".repeat(128 * 1024));
        let used = format!("{long}{}</r>", "<a p:x=\"1\"/>".repeat(30_000));
        assert_eq!(parse(&used).unwrap_err(), OVER_BUDGET);
        // An ordinary assertion's declarations are far inside the budget.
        let some: String = (0..32)
            .map(|i| format!(" xmlns:p{i}=\"urn:{i}\""))
            .collect();
        let few = format!("<r{some}>{}</r>", "<a xmlns:q=\"urn:q\"/>".repeat(20));
        assert!(parse(&few).is_ok());
    }

    #[test]
    fn exclusive_c14n_basics() {
        // Empty elements expand, attributes sort by namespace then name, unused namespaces drop,
        // and redundant declarations are not repeated.
        assert_eq!(
            c14n(
                r#"<a:r xmlns:a="urn:a" xmlns:b="urn:b" z="1" b:y="2" a="3"><a:c xmlns:a="urn:a"/><d xmlns="urn:d">x &amp; &#x41;</d></a:r>"#
            ),
            r#"<a:r xmlns:a="urn:a" xmlns:b="urn:b" a="3" z="1" b:y="2"><a:c></a:c><d xmlns="urn:d">x &amp; A</d></a:r>"#
        );
        assert_eq!(
            c14n("<r xmlns:u=\"urn:u\" v=\"a\tb\">\r\n&gt;</r>"),
            "<r v=\"a b\">\n&gt;</r>"
        );
        assert_eq!(
            c14n(r#"<r xmlns="urn:x"><s xmlns=""/></r>"#),
            r#"<r xmlns="urn:x"><s xmlns=""></s></r>"#
        );
    }

    /// Review finding: with `#default` in the InclusiveNamespaces PrefixList, an element whose
    /// default namespace is undeclared under an output ancestor with a non-empty default must
    /// render `xmlns=""`, though it does not use the default namespace itself.
    #[test]
    fn inclusive_default_namespace_keeps_its_undeclaration() {
        let xml = r#"<r xmlns="urn:x"><p:s xmlns:p="urn:p" xmlns=""><p:t/></p:s></r>"#;
        let doc = Document::parse(xml).unwrap();
        let render = |inclusive: &[String]| doc.canonicalize(doc.root(), None, inclusive).unwrap();
        assert_eq!(
            render(&[String::new()]),
            r#"<r xmlns="urn:x"><p:s xmlns="" xmlns:p="urn:p"><p:t></p:t></p:s></r>"#
        );
        // Without it, the default namespace is not visibly utilized by p:s and is left out.
        assert_eq!(
            render(&[]),
            r#"<r xmlns="urn:x"><p:s xmlns:p="urn:p"><p:t></p:t></p:s></r>"#
        );
        // Nothing to undeclare when no output ancestor rendered a default.
        let doc = Document::parse(r#"<p:s xmlns:p="urn:p" xmlns=""/>"#).unwrap();
        let out = doc
            .canonicalize(doc.root(), None, &[String::new()])
            .unwrap();
        assert_eq!(out, r#"<p:s xmlns:p="urn:p"></p:s>"#);
    }

    /// Review finding: each step of a verification had a bound of its own, and each round found
    /// a step whose bound missed a case. Parsing and both canonicalizations now draw on one
    /// budget fixed by the document's size: a document small enough to parse is still refused
    /// when rendering it would weigh many inclusive prefixes at each of many elements.
    #[test]
    fn parsing_and_canonicalization_share_one_budget() {
        let xml = format!("<r>{}</r>", "<a/>".repeat(60_000));
        let doc = Document::parse(&xml).unwrap();
        let prefixes: Vec<String> = (0..MAX_INCLUSIVE_PREFIXES)
            .map(|i| format!("p{i}"))
            .collect();
        assert_eq!(
            doc.canonicalize(doc.root(), None, &prefixes).unwrap_err(),
            OVER_BUDGET
        );
        // What one canonicalization spends is gone for the next.
        let doc = Document::parse(&xml).unwrap();
        let mut rounds = 0;
        while doc.canonicalize(doc.root(), None, &[]).is_ok() {
            rounds += 1;
            assert!(rounds < 100, "the budget was never spent");
        }
    }

    /// Review finding: comparing an inherited namespace URI with its rendered copy cost its bytes
    /// at every element, uncharged, so a long URI named as an inclusive prefix and thousands of
    /// small elements cost gigabytes of comparisons. Lookups, comparisons and copies are charged
    /// by the bytes they weigh.
    #[test]
    fn namespace_comparisons_draw_on_the_budget() {
        let xml = format!(
            "<r xmlns:p=\"urn:{}\">{}</r>",
            "x".repeat(100_000),
            "<a/>".repeat(30_000)
        );
        let doc = Document::parse(&xml).unwrap();
        assert_eq!(
            doc.canonicalize(doc.root(), None, &["p".to_string()])
                .unwrap_err(),
            OVER_BUDGET
        );
        let long = "q".repeat(100_000);
        let xml = format!("<r xmlns:{long}=\"urn:q\">{}</r>", "<a/>".repeat(30_000));
        let doc = Document::parse(&xml).unwrap();
        assert_eq!(
            doc.canonicalize(doc.root(), None, &[long]).unwrap_err(),
            OVER_BUDGET
        );
    }

    /// Review finding: attributes were sorted by their namespace URIs uncharged, so many
    /// attributes sharing one long URI cost far more to sort than to parse. Sorting is charged
    /// for the comparisons it can make, before it is done.
    #[test]
    fn sorting_attributes_draws_on_the_budget() {
        let mut attrs: Vec<String> = (0..400).map(|i| format!(" p:a{i}=\"1\"")).collect();
        attrs.reverse();
        let xml = format!(
            "<r xmlns:p=\"urn:{}\"{}/>",
            "x".repeat(2048),
            attrs.concat()
        );
        let doc = Document::parse(&xml).unwrap();
        assert_eq!(
            doc.canonicalize(doc.root(), None, &[]).unwrap_err(),
            OVER_BUDGET
        );
        // A handful of attributes in one namespace still sort within it.
        let xml = r#"<r xmlns:p="urn:p" p:b="1" p:a="2" c="3"/>"#;
        let doc = Document::parse(xml).unwrap();
        assert_eq!(
            doc.canonicalize(doc.root(), None, &[]).unwrap(),
            r#"<r xmlns:p="urn:p" c="3" p:a="2" p:b="1"></r>"#
        );
    }

    /// The bytes written are charged before they are written, so the charge is what escaping
    /// writes, exactly.
    #[test]
    fn escaped_lengths_are_what_escaping_writes() {
        let s = "a&b<c>d\"e\tf\ng\rh\u{e9}\u{1f600}";
        let mut out = String::new();
        escape_text(s, &mut out);
        assert_eq!(escaped_len(s, false), out.len());
        out.clear();
        escape_attr(s, &mut out);
        assert_eq!(escaped_len(s, true), out.len());
    }

    /// Review finding: relative namespace URIs were accepted, though canonicalization must fail
    /// on them (Canonical XML 1.0 section 2.1), unused ones included.
    #[test]
    fn relative_namespaces_are_refused() {
        for xml in [
            r#"<r xmlns:unused="relative"/>"#,
            r#"<r xmlns="relative"/>"#,
            r#"<r xmlns:p="../x"/>"#,
            r#"<r xmlns:p="1urn:x"/>"#,
        ] {
            assert!(parse(xml).is_err(), "{xml}");
        }
        for xml in [
            r#"<r xmlns:p="urn:x"/>"#,
            r#"<r xmlns="http://e.example/ns"/>"#,
            r#"<r xmlns=""/>"#,
        ] {
            assert!(parse(xml).is_ok(), "{xml}");
        }
    }

    /// Review finding: canonicalization copied the rendered namespaces at every element that
    /// rendered one, and weighed every inclusive prefix a transform named at every element. The
    /// prefixes are capped, and one map is extended and taken back instead of copied, so a
    /// declaration an element renders reaches its descendants and none of its siblings.
    #[test]
    fn canonicalization_work_is_bounded() {
        let method = |n: usize| {
            let list: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
            let xml = format!(
                r#"<m xmlns="{EXC_C14N}"><InclusiveNamespaces PrefixList="{} p0"/></m>"#,
                list.join(" ")
            );
            inclusive_prefixes(&parse(&xml).unwrap())
        };
        assert_eq!(
            method(MAX_INCLUSIVE_PREFIXES).unwrap().len(),
            MAX_INCLUSIVE_PREFIXES
        );
        assert!(method(MAX_INCLUSIVE_PREFIXES + 1).is_err());
        assert_eq!(
            c14n(
                r#"<r xmlns:p="urn:p"><p:a><p:c/></p:a><p:b/><s xmlns:p="urn:q"><p:d/></s><p:e/></r>"#
            ),
            concat!(
                r#"<r><p:a xmlns:p="urn:p"><p:c></p:c></p:a><p:b xmlns:p="urn:p"></p:b>"#,
                r#"<s><p:d xmlns:p="urn:q"></p:d></s><p:e xmlns:p="urn:p"></p:e></r>"#
            )
        );
        let decls: String = (0..1024)
            .map(|i| format!(" xmlns:p{i}=\"urn:{i}\" p{i}:a=\"1\""))
            .collect();
        // The root renders a thousand declarations; each child renders one more of its own.
        let wide = format!("<r xmlns:q=\"urn:q\"{decls}>{}</r>", "<q:x/>".repeat(5_000));
        let out = c14n(&wide);
        assert_eq!(out.matches("xmlns:q=").count(), 5_000);
    }

    /// Review finding: repeated attributes were found by comparing each attribute with every one
    /// before it, twice (quick-xml's check, then the expanded names), so one unsigned element
    /// with tens of thousands of attributes cost hundreds of millions of comparisons. Both checks
    /// are sets now, and still refuse a repeat.
    #[test]
    fn attribute_checks_are_linear() {
        let attrs: String = (0..20_000).map(|i| format!(" a{i}=\"1\"")).collect();
        assert_eq!(parse(&format!("<r{attrs}/>")).unwrap().attrs.len(), 20_000);
        assert!(parse(r#"<r a="1" b="2" a="3"/>"#).is_err());
        assert!(parse(r#"<r xmlns:p="urn:p" xmlns:p="urn:q"/>"#).is_err());
        assert!(parse(r#"<r xmlns:p="urn:u" xmlns:q="urn:u" p:a="1" q:a="2"/>"#).is_err());
        assert!(parse(r#"<r xmlns:p="urn:u" xmlns:q="urn:v" p:a="1" q:a="2"/>"#).is_ok());
    }

    #[test]
    fn hostile_xml_is_refused() {
        assert!(parse(r#"<!DOCTYPE r [<!ENTITY x "y">]><r>&x;</r>"#).is_err());
        assert!(parse("<r><?pi x?></r>").is_err());
        assert!(parse("<r>&unknown;</r>").is_err());
        assert!(parse("<r></r><s/>").is_err());
        assert!(parse("<p:r/>").is_err());
    }

    /// Build and sign an assertion the way the Touchstone identity provider does, with an
    /// ecdsa-sha256 key.
    fn signed_assertion(
        key: &jose::EcKey,
        issuer: &str,
        audience: &str,
        name_id: &str,
        tamper: bool,
    ) -> String {
        signed_assertion_to(
            key,
            issuer,
            audience,
            name_id,
            Some("https://client.example/"),
            tamper,
        )
    }

    /// [`signed_assertion`] with a chosen `Recipient` (none when `None`).
    fn signed_assertion_to(
        key: &jose::EcKey,
        issuer: &str,
        audience: &str,
        name_id: &str,
        recipient: Option<&str>,
        tamper: bool,
    ) -> String {
        let recipient = recipient
            .map(|r| format!(" Recipient=\"{r}\""))
            .unwrap_or_default();
        let now = jose::now_secs();
        let confirmation = format!(
            "<saml:SubjectConfirmation Method=\"{BEARER}\"><saml:SubjectConfirmationData NotOnOrAfter=\"{}\"{recipient}/></saml:SubjectConfirmation>",
            instant(now + 300)
        );
        let restriction = format!(
            "<saml:AudienceRestriction><saml:Audience>{audience}</saml:Audience></saml:AudienceRestriction>"
        );
        signed_custom(key, issuer, name_id, &confirmation, &restriction, tamper)
    }

    /// An xsd:dateTime for `t` (Unix seconds).
    fn instant(t: i64) -> String {
        let days = t.div_euclid(86_400);
        let secs = t.rem_euclid(86_400);
        // civil from days (Hinnant)
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            secs / 3600,
            secs % 3600 / 60,
            secs % 60
        )
    }

    /// A signed assertion with the given `SubjectConfirmation` elements and `Conditions`
    /// children (the conditions valid for five minutes from now).
    fn signed_custom(
        key: &jose::EcKey,
        issuer: &str,
        name_id: &str,
        confirmations: &str,
        conditions: &str,
        tamper: bool,
    ) -> String {
        let now = jose::now_secs();
        let id = "_abc123";
        let body = |name: &str| {
            format!(
                "<saml:Issuer>{issuer}</saml:Issuer>{{SIG}}<saml:Subject><saml:NameID Format=\"urn:oasis:names:tc:SAML:2.0:nameid-format:persistent\">{name}</saml:NameID>{confirmations}</saml:Subject><saml:Conditions NotBefore=\"{nb}\" NotOnOrAfter=\"{exp}\">{conditions}</saml:Conditions>",
                exp = instant(now + 300),
                nb = instant(now)
            )
        };
        let open = format!("<saml:Assertion xmlns:saml=\"{SAML_NS}\" ID=\"{id}\" IssueInstant=\"{}\" Version=\"2.0\">", instant(now));
        let unsigned = format!(
            "{open}{}</saml:Assertion>",
            body(name_id).replace("{SIG}", "")
        );
        let digest = STANDARD.encode(sha2::Sha256::digest(c14n(&unsigned).as_bytes()));
        let signed_info = format!(
            "<ds:SignedInfo><ds:CanonicalizationMethod Algorithm=\"{EXC_C14N}\"/><ds:SignatureMethod Algorithm=\"{ECDSA_SHA256}\"/><ds:Reference URI=\"#{id}\"><ds:Transforms><ds:Transform Algorithm=\"{ENVELOPED}\"/><ds:Transform Algorithm=\"{EXC_C14N}\"/></ds:Transforms><ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"/><ds:DigestValue>{digest}</ds:DigestValue></ds:Reference></ds:SignedInfo>"
        );
        // SignedInfo canonicalizes with the ds namespace declared on it.
        let si_c14n = c14n(&signed_info.replacen(
            "<ds:SignedInfo>",
            &format!("<ds:SignedInfo xmlns:ds=\"{DS_NS}\">"),
            1,
        ));
        let sig = STANDARD.encode(key.sign(si_c14n.as_bytes()));
        let signature = format!("<ds:Signature xmlns:ds=\"{DS_NS}\">{signed_info}<ds:SignatureValue>{sig}</ds:SignatureValue></ds:Signature>");
        let final_name = if tamper {
            format!("{name_id}#altered")
        } else {
            name_id.to_string()
        };
        let xml = format!(
            "{open}{}</saml:Assertion>",
            body(&final_name).replace("{SIG}", &signature)
        );
        URL_SAFE_NO_PAD.encode(xml)
    }

    #[test]
    fn signed_assertions_verify() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        let key = jose::EcKey::generate("idp");
        let idp = "https://idp.example/";
        cfg.saml_idps
            .insert(idp.into(), key.public_jwk().to_string());
        let ok = signed_assertion(&key, idp, cfg.issuer(), "https://alice.example/#me", false);
        let v = verify(&cfg, &ok).unwrap();
        assert_eq!(v.subject, "https://alice.example/#me");
        assert_eq!(v.client, "https://client.example/");
        // Altered after signing, foreign audience, untrusted issuer, wrong key.
        assert!(verify(
            &cfg,
            &signed_assertion(&key, idp, cfg.issuer(), "https://alice.example/#me", true)
        )
        .is_err());
        assert!(verify(
            &cfg,
            &signed_assertion(
                &key,
                idp,
                "https://other.example",
                "https://alice.example/#me",
                false
            )
        )
        .is_err());
        assert!(verify(
            &cfg,
            &signed_assertion(&key, "https://rogue.example/", cfg.issuer(), "a", false)
        )
        .is_err());
        let other = jose::EcKey::generate("x");
        assert!(verify(
            &cfg,
            &signed_assertion(&other, idp, cfg.issuer(), "a", false)
        )
        .is_err());
        // Expired.
        assert!(verify_at(&cfg, &ok, jose::now_secs() + 3600).is_err());
    }

    #[test]
    fn subject_and_recipient_must_be_uris() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        let key = jose::EcKey::generate("idp");
        let idp = "https://idp.example/";
        cfg.saml_idps
            .insert(idp.into(), key.public_jwk().to_string());
        let alice = "https://alice.example/#me";
        // A NameID that is not a URI.
        let err = verify(
            &cfg,
            &signed_assertion(&key, idp, cfg.issuer(), "alice", false),
        );
        assert!(err.unwrap_err().contains("NameID"));
        // No Recipient: refused, not replaced by the issuer.
        let none = signed_assertion_to(&key, idp, cfg.issuer(), alice, None, false);
        assert!(verify(&cfg, &none).unwrap_err().contains("Recipient"));
        // A Recipient that is not a URI.
        let bad = signed_assertion_to(&key, idp, cfg.issuer(), alice, Some("client"), false);
        assert!(verify(&cfg, &bad).unwrap_err().contains("Recipient"));
    }

    fn saml_cfg() -> (LwsConfig, jose::EcKey, &'static str) {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        let key = jose::EcKey::generate("idp");
        let idp = "https://idp.example/";
        cfg.saml_idps
            .insert(idp.into(), key.public_jwk().to_string());
        (cfg, key, idp)
    }

    fn confirmation(method: &str, data: &str) -> String {
        format!("<saml:SubjectConfirmation Method=\"{method}\">{data}</saml:SubjectConfirmation>")
    }

    fn confirmation_data(attrs: &str) -> String {
        format!("<saml:SubjectConfirmationData{attrs}/>")
    }

    #[test]
    fn only_bearer_subject_confirmations_are_accepted() {
        let (cfg, key, idp) = saml_cfg();
        let alice = "https://alice.example/#me";
        let now = jose::now_secs();
        let exp = instant(now + 300);
        let ours = format!(
            "<saml:AudienceRestriction><saml:Audience>{}</saml:Audience></saml:AudienceRestriction>",
            cfg.issuer()
        );
        let good = confirmation_data(&format!(
            " NotOnOrAfter=\"{exp}\" Recipient=\"https://client.example/\""
        ));
        let verify_with = |confirmations: &str| {
            verify(
                &cfg,
                &signed_custom(&key, idp, alice, confirmations, &ours, false),
            )
        };
        assert_eq!(
            verify_with(&confirmation(BEARER, &good)).unwrap().client,
            "https://client.example/"
        );
        // Holder-of-key and sender-vouches carry no proof here: refused, alone or beside a bearer.
        for method in [
            "urn:oasis:names:tc:SAML:2.0:cm:holder-of-key",
            "urn:oasis:names:tc:SAML:2.0:cm:sender-vouches",
        ] {
            let err = verify_with(&confirmation(method, &good)).unwrap_err();
            assert!(err.contains("not accepted"), "{err}");
            let both = format!(
                "{}{}",
                confirmation(BEARER, &good),
                confirmation(method, &good)
            );
            assert!(verify_with(&both).is_err());
        }
        // No confirmation, or one without a method.
        assert!(verify_with("").unwrap_err().contains("no bearer"));
        assert!(verify_with(
            "<saml:SubjectConfirmation><saml:SubjectConfirmationData/></saml:SubjectConfirmation>"
        )
        .is_err());
        // Bearer without data, without NotOnOrAfter, expired, or not yet valid.
        assert!(verify_with(&confirmation(BEARER, "")).is_err());
        let no_limit = confirmation_data(" Recipient=\"https://client.example/\"");
        assert!(verify_with(&confirmation(BEARER, &no_limit))
            .unwrap_err()
            .contains("NotOnOrAfter"));
        let expired = confirmation_data(&format!(
            " NotOnOrAfter=\"{}\" Recipient=\"https://client.example/\"",
            instant(now - 3600)
        ));
        assert!(verify_with(&confirmation(BEARER, &expired))
            .unwrap_err()
            .contains("expired"));
        let early = confirmation_data(&format!(
            " NotBefore=\"{}\" NotOnOrAfter=\"{exp}\" Recipient=\"https://client.example/\"",
            instant(now + 3600)
        ));
        assert!(verify_with(&confirmation(BEARER, &early)).is_err());
        // Two bearer confirmations naming different clients.
        let other = confirmation_data(&format!(
            " NotOnOrAfter=\"{exp}\" Recipient=\"https://other.example/\""
        ));
        let two = format!(
            "{}{}",
            confirmation(BEARER, &good),
            confirmation(BEARER, &other)
        );
        assert!(verify_with(&two).unwrap_err().contains("different"));
    }

    #[test]
    fn every_audience_restriction_must_name_this_server() {
        let (cfg, key, idp) = saml_cfg();
        let alice = "https://alice.example/#me";
        let now = jose::now_secs();
        let bearer = confirmation(
            BEARER,
            &confirmation_data(&format!(
                " NotOnOrAfter=\"{}\" Recipient=\"https://client.example/\"",
                instant(now + 300)
            )),
        );
        let restriction = |audiences: &[&str]| {
            let inner: String = audiences
                .iter()
                .map(|a| format!("<saml:Audience>{a}</saml:Audience>"))
                .collect();
            format!("<saml:AudienceRestriction>{inner}</saml:AudienceRestriction>")
        };
        let us = cfg.issuer().to_string();
        let verify_with = |conditions: &str| {
            verify(
                &cfg,
                &signed_custom(&key, idp, alice, &bearer, conditions, false),
            )
        };
        // One restriction listing several audiences, ours among them: accepted.
        assert!(verify_with(&restriction(&["https://other.example", &us])).is_ok());
        // Two restrictions, both naming us: accepted.
        let both = format!("{}{}", restriction(&[&us]), restriction(&[&us, "x:y"]));
        assert!(verify_with(&both).is_ok());
        // Two restrictions, one of which does not name us: the assertion is not for us.
        let split = format!(
            "{}{}",
            restriction(&[&us]),
            restriction(&["https://other.example"])
        );
        assert!(verify_with(&split)
            .unwrap_err()
            .contains("AudienceRestriction"));
        // An Audience nested deeper than a direct AudienceRestriction does not count.
        let nested = format!(
            "{}<saml:ProxyRestriction><saml:Audience>{us}</saml:Audience></saml:ProxyRestriction>",
            restriction(&["https://other.example"])
        );
        assert!(verify_with(&nested).is_err());
        // No restriction at all.
        assert!(verify_with("").is_err());
        // ProxyRestriction is ignored; OneTimeUse and unknown conditions are refused.
        let proxy = format!(
            "{}<saml:ProxyRestriction Count=\"0\"/>",
            restriction(&[&us])
        );
        assert!(verify_with(&proxy).is_ok());
        let once = format!("{}<saml:OneTimeUse/>", restriction(&[&us]));
        assert!(verify_with(&once).unwrap_err().contains("OneTimeUse"));
        let unknown = format!(
            "{}<saml:Condition xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"x:Custom\"/>",
            restriction(&[&us])
        );
        assert!(verify_with(&unknown)
            .unwrap_err()
            .contains("does not understand"));
        let foreign = format!("{}<x:Whatever xmlns:x=\"urn:x\"/>", restriction(&[&us]));
        assert!(verify_with(&foreign).is_err());
    }

    #[test]
    fn trust_keys_parse() {
        let key = jose::EcKey::generate("k");
        assert!(matches!(
            TrustKey::parse(&key.public_jwk().to_string()).unwrap(),
            TrustKey::P256(_)
        ));
        // A P-256 SubjectPublicKeyInfo.
        let point =
            p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(&key.public_key(), false);
        let mut spki = vec![0x30, 0x59, 0x30, 0x13, 0x06, 0x07];
        spki.extend_from_slice(EC_OID);
        spki.extend_from_slice(&[0x06, 0x08]);
        spki.extend_from_slice(P256_OID);
        spki.extend_from_slice(&[0x03, 0x42, 0x00]);
        spki.extend_from_slice(point.as_bytes());
        let pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            STANDARD.encode(&spki)
        );
        assert!(matches!(TrustKey::parse(&pem).unwrap(), TrustKey::P256(_)));
        assert!(TrustKey::parse("garbage").is_err());
    }
}
